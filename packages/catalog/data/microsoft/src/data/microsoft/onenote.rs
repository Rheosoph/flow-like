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
// OneNote Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OneNoteNotebook {
    pub id: String,
    pub display_name: String,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub is_default: Option<bool>,
    pub user_role: Option<String>,
    pub links_one_note_client_url: Option<String>,
    pub links_one_note_web_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OneNoteSection {
    pub id: String,
    pub display_name: String,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub is_default: Option<bool>,
    pub parent_notebook_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OneNotePage {
    pub id: String,
    pub title: String,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub content_url: Option<String>,
    pub parent_section_id: Option<String>,
}

fn parse_notebook(value: &Value) -> Option<OneNoteNotebook> {
    Some(OneNoteNotebook {
        id: value["id"].as_str()?.to_string(),
        display_name: value["displayName"].as_str()?.to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: value["lastModifiedDateTime"].as_str().map(String::from),
        is_default: value["isDefault"].as_bool(),
        user_role: value["userRole"].as_str().map(String::from),
        links_one_note_client_url: value["links"]["oneNoteClientUrl"]["href"]
            .as_str()
            .map(String::from),
        links_one_note_web_url: value["links"]["oneNoteWebUrl"]["href"]
            .as_str()
            .map(String::from),
    })
}

fn parse_section(value: &Value) -> Option<OneNoteSection> {
    Some(OneNoteSection {
        id: value["id"].as_str()?.to_string(),
        display_name: value["displayName"].as_str()?.to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: value["lastModifiedDateTime"].as_str().map(String::from),
        is_default: value["isDefault"].as_bool(),
        parent_notebook_id: value["parentNotebook"]["id"].as_str().map(String::from),
    })
}

fn parse_page(value: &Value) -> Option<OneNotePage> {
    Some(OneNotePage {
        id: value["id"].as_str()?.to_string(),
        title: value["title"].as_str()?.to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: value["lastModifiedDateTime"].as_str().map(String::from),
        content_url: value["contentUrl"].as_str().map(String::from),
        parent_section_id: value["parentSection"]["id"].as_str().map(String::from),
    })
}

// =============================================================================
// List Notebooks Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListNotebooksNode {}

impl ListNotebooksNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListNotebooksNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_list_notebooks",
            "List Notebooks",
            "List all OneNote notebooks",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "listNotebooks");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

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
        node.add_output_pin("notebooks", "Notebooks", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<OneNoteNotebook>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url("/me/onenote/notebooks"),
        )
        .await
        {
            Ok(values) => {
                let notebooks: Vec<OneNoteNotebook> =
                    values.iter().filter_map(parse_notebook).collect();
                let count = notebooks.len() as i64;
                context.set_pin_value("notebooks", json!(notebooks)).await?;
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
// Create Notebook Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateNotebookNode {}

impl CreateNotebookNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateNotebookNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_create_notebook",
            "Create Notebook",
            "Create a new OneNote notebook",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "createNotebook");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

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
            "Name of the notebook",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("notebook", "Notebook", "", VariableType::Struct)
            .set_schema::<OneNoteNotebook>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let display_name: String = context.evaluate_pin("display_name").await?;

        let request_body = json!({
            "displayName": display_name
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/me/onenote/notebooks"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(notebook) = parse_notebook(&body) {
                    context.set_pin_value("notebook", json!(notebook)).await?;
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

// =============================================================================
// List Sections Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListSectionsNode {}

impl ListSectionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListSectionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_list_sections",
            "List Sections",
            "List all sections in a OneNote notebook",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "listSections");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

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
            "notebook_id",
            "Notebook ID",
            "ID of the notebook",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("sections", "Sections", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<OneNoteSection>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let notebook_id: String = context.evaluate_pin("notebook_id").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&format!("/me/onenote/notebooks/{}/sections", notebook_id)),
        )
        .await
        {
            Ok(values) => {
                let sections: Vec<OneNoteSection> =
                    values.iter().filter_map(parse_section).collect();
                let count = sections.len() as i64;
                context.set_pin_value("sections", json!(sections)).await?;
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
// Create Section Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateSectionNode {}

impl CreateSectionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateSectionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_create_section",
            "Create Section",
            "Create a new section in a OneNote notebook",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "createSection");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

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
            "notebook_id",
            "Notebook ID",
            "ID of the notebook",
            VariableType::String,
        );
        node.add_input_pin(
            "display_name",
            "Display Name",
            "Name of the section",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("section", "Section", "", VariableType::Struct)
            .set_schema::<OneNoteSection>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let notebook_id: String = context.evaluate_pin("notebook_id").await?;
        let display_name: String = context.evaluate_pin("display_name").await?;

        let body = json!({
            "displayName": display_name
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url(&format!("/me/onenote/notebooks/{}/sections", notebook_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(section) = parse_section(&body) {
                    context.set_pin_value("section", json!(section)).await?;
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

// =============================================================================
// List Pages Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListPagesNode {}

impl ListPagesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListPagesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_list_pages",
            "List Pages",
            "List all pages in a OneNote section",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "listPages");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

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
            "section_id",
            "Section ID",
            "ID of the section",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("pages", "Pages", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<OneNotePage>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let section_id: String = context.evaluate_pin("section_id").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&format!("/me/onenote/sections/{}/pages", section_id)),
        )
        .await
        {
            Ok(values) => {
                let pages: Vec<OneNotePage> = values.iter().filter_map(parse_page).collect();
                let count = pages.len() as i64;
                context.set_pin_value("pages", json!(pages)).await?;
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
// Create Page Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreatePageNode {}

impl CreatePageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreatePageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_create_page",
            "Create Page",
            "Create a new page in a OneNote section",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "createPage");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

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
            "section_id",
            "Section ID",
            "ID of the section",
            VariableType::String,
        );
        node.add_input_pin("title", "Title", "Page title", VariableType::String);
        node.add_input_pin(
            "content",
            "Content",
            "HTML content for the page body",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("page", "Page", "", VariableType::Struct)
            .set_schema::<OneNotePage>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let section_id: String = context.evaluate_pin("section_id").await?;
        let title: String = context.evaluate_pin("title").await?;
        let content: String = context.evaluate_pin("content").await.unwrap_or_default();

        let html_content = format!(
            r#"<!DOCTYPE html>
<html>
<head>
<title>{}</title>
</head>
<body>
{}
</body>
</html>"#,
            title, content
        );

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url(&format!("/me/onenote/sections/{}/pages", section_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/xhtml+xml")
            .body(html_content)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(page) = parse_page(&body) {
                    context.set_pin_value("page", json!(page)).await?;
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

// =============================================================================
// Get Page Content Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetPageContentNode {}

impl GetPageContentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetPageContentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_get_page_content",
            "Get Page Content",
            "Get the HTML content of a OneNote page",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "getPageContent");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("page_id", "Page ID", "ID of the page", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "content",
            "Content",
            "HTML content of the page",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(provider.api_url(&format!("/me/onenote/pages/{}/content", page_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let content = resp.text().await?;
                context.set_pin_value("content", json!(content)).await?;
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
// Delete Page Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeletePageNode {}

impl DeletePageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeletePageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_onenote_delete_page",
            "Delete Page",
            "Delete a OneNote page",
            "Data/Microsoft/OneNote",
        );
        node.set_flowscript_name("microsoft.onenote", "deletePage");
        node.set_version(1);
        node.add_icon("/flow/icons/onenote.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("page_id", "Page ID", "ID of the page", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Notes.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .delete(provider.api_url(&format!("/me/onenote/pages/{}", page_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 204 => {
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
