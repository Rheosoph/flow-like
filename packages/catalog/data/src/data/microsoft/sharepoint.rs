use super::provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// SharePoint Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharePointSite {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub web_url: String,
    pub description: Option<String>,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
}

fn parse_site(site: &Value) -> Option<SharePointSite> {
    Some(SharePointSite {
        id: site["id"].as_str()?.to_string(),
        name: site["name"].as_str()?.to_string(),
        display_name: site["displayName"].as_str()?.to_string(),
        web_url: site["webUrl"].as_str()?.to_string(),
        description: site["description"].as_str().map(String::from),
        created_date_time: site["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: site["lastModifiedDateTime"].as_str().map(String::from),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharePointDrive {
    pub id: String,
    pub name: String,
    pub drive_type: String,
    pub web_url: String,
    pub description: Option<String>,
    pub quota_total: Option<i64>,
    pub quota_used: Option<i64>,
    pub quota_remaining: Option<i64>,
}

fn parse_drive(drive: &Value) -> Option<SharePointDrive> {
    Some(SharePointDrive {
        id: drive["id"].as_str()?.to_string(),
        name: drive["name"].as_str()?.to_string(),
        drive_type: drive["driveType"].as_str()?.to_string(),
        web_url: drive["webUrl"].as_str()?.to_string(),
        description: drive["description"].as_str().map(String::from),
        quota_total: drive["quota"]["total"].as_i64(),
        quota_used: drive["quota"]["used"].as_i64(),
        quota_remaining: drive["quota"]["remaining"].as_i64(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharePointDriveItem {
    pub id: String,
    pub name: String,
    pub web_url: String,
    pub size: Option<i64>,
    pub is_folder: bool,
    pub mime_type: Option<String>,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub download_url: Option<String>,
    pub parent_path: Option<String>,
}

fn parse_drive_item(item: &Value) -> Option<SharePointDriveItem> {
    let is_folder = item.get("folder").is_some();
    Some(SharePointDriveItem {
        id: item["id"].as_str()?.to_string(),
        name: item["name"].as_str()?.to_string(),
        web_url: item["webUrl"].as_str()?.to_string(),
        size: item["size"].as_i64(),
        is_folder,
        mime_type: item["file"]["mimeType"].as_str().map(String::from),
        created_date_time: item["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: item["lastModifiedDateTime"].as_str().map(String::from),
        download_url: item["@microsoft.graph.downloadUrl"]
            .as_str()
            .map(String::from),
        parent_path: item["parentReference"]["path"].as_str().map(String::from),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharePointList {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub web_url: String,
    pub description: Option<String>,
    pub list_template: Option<String>,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
}

fn parse_list(list: &Value) -> Option<SharePointList> {
    Some(SharePointList {
        id: list["id"].as_str()?.to_string(),
        name: list["name"].as_str()?.to_string(),
        display_name: list["displayName"].as_str()?.to_string(),
        web_url: list["webUrl"].as_str()?.to_string(),
        description: list["description"].as_str().map(String::from),
        list_template: list["list"]["template"].as_str().map(String::from),
        created_date_time: list["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: list["lastModifiedDateTime"].as_str().map(String::from),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SharePointListItem {
    pub id: String,
    pub web_url: String,
    pub created_date_time: Option<String>,
    pub last_modified_date_time: Option<String>,
    pub fields: Value,
}

fn parse_list_item(item: &Value) -> Option<SharePointListItem> {
    Some(SharePointListItem {
        id: item["id"].as_str()?.to_string(),
        web_url: item["webUrl"].as_str()?.to_string(),
        created_date_time: item["createdDateTime"].as_str().map(String::from),
        last_modified_date_time: item["lastModifiedDateTime"].as_str().map(String::from),
        fields: item["fields"].clone(),
    })
}

// =============================================================================
// Search Sites Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SearchSharePointSitesNode {}

impl SearchSharePointSitesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchSharePointSitesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_search_sites",
            "Search SharePoint Sites",
            "Search for SharePoint sites by keyword",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

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
            "Search Query",
            "Search term to find sites",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "sites",
            "Sites",
            "List of matching SharePoint sites",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SharePointSite>();
        node.add_output_pin(
            "count",
            "Count",
            "Number of sites found",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.set_long_running(true);
        node.set_scores(NodeScores::new().set_privacy(5).set_security(7).build());
        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let query: String = context.evaluate_pin("query").await?;

        let url = format!(
            "{}/sites?search={}",
            provider.base_url,
            urlencoding::encode(&query)
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
                let sites = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_site).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = sites.len() as i64;
                context.set_pin_value("sites", json!(sites)).await?;
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
// Get Site Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetSharePointSiteNode {}

impl GetSharePointSiteNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSharePointSiteNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_get_site",
            "Get SharePoint Site",
            "Get a SharePoint site by hostname and path or site ID",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

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
            "hostname",
            "Hostname",
            "SharePoint hostname (e.g., 'contoso.sharepoint.com')",
            VariableType::String,
        );
        node.add_input_pin(
            "site_path",
            "Site Path",
            "Site path (e.g., '/sites/marketing')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "site_id",
            "Site ID",
            "Alternatively, provide the site ID directly",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("site", "Site", "SharePoint site", VariableType::Struct)
            .set_schema::<SharePointSite>();
        node.add_output_pin("resolved_site_id", "Site ID", "The site ID", VariableType::String);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let hostname: String = context.evaluate_pin("hostname").await.unwrap_or_default();
        let site_path: String = context.evaluate_pin("site_path").await.unwrap_or_default();
        let site_id: String = context.evaluate_pin("site_id").await.unwrap_or_default();

        let url = if !site_id.is_empty() {
            format!("{}/sites/{}", provider.base_url, site_id)
        } else if !hostname.is_empty() {
            if site_path.is_empty() {
                format!("{}/sites/{}:", provider.base_url, hostname)
            } else {
                format!("{}/sites/{}:{}", provider.base_url, hostname, site_path)
            }
        } else {
            context
                .set_pin_value(
                    "error_message",
                    json!("Either hostname or site_id is required"),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
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
                if let Some(site) = parse_site(&body) {
                    let id = site.id.clone();
                    context.set_pin_value("site", json!(site)).await?;
                    context.set_pin_value("resolved_site_id", json!(id)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse site response"))
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
// List Site Drives Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListSharePointDrivesNode {}

impl ListSharePointDrivesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListSharePointDrivesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_list_drives",
            "List SharePoint Drives",
            "List document libraries (drives) in a SharePoint site",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

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
            "site_id",
            "Site ID",
            "SharePoint site ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "drives",
            "Drives",
            "List of document libraries",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SharePointDrive>();
        node.add_output_pin("count", "Count", "Number of drives", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let site_id: String = context.evaluate_pin("site_id").await?;

        let url = format!("{}/sites/{}/drives", provider.base_url, site_id);

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
                let drives = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_drive).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = drives.len() as i64;
                context.set_pin_value("drives", json!(drives)).await?;
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
// List Drive Items Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListSharePointDriveItemsNode {}

impl ListSharePointDriveItemsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListSharePointDriveItemsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_list_drive_items",
            "List Drive Items",
            "List files and folders in a SharePoint drive (document library)",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

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
            "drive_id",
            "Drive ID",
            "Drive (document library) ID",
            VariableType::String,
        );
        node.add_input_pin(
            "folder_path",
            "Folder Path",
            "Path to folder (empty for root, e.g., '/Documents/Reports')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "folder_id",
            "Folder ID",
            "Alternatively, provide folder item ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "items",
            "Items",
            "List of files and folders",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SharePointDriveItem>();
        node.add_output_pin("count", "Count", "Number of items", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let folder_path: String = context
            .evaluate_pin("folder_path")
            .await
            .unwrap_or_default();
        let folder_id: String = context.evaluate_pin("folder_id").await.unwrap_or_default();

        let url = if !folder_id.is_empty() {
            format!(
                "{}/drives/{}/items/{}/children",
                provider.base_url, drive_id, folder_id
            )
        } else if !folder_path.is_empty() {
            format!(
                "{}/drives/{}/root:{}:/children",
                provider.base_url, drive_id, folder_path
            )
        } else {
            format!("{}/drives/{}/root/children", provider.base_url, drive_id)
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
                let items = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_drive_item).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = items.len() as i64;
                context.set_pin_value("items", json!(items)).await?;
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
// Download File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DownloadSharePointFileNode {}

impl DownloadSharePointFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DownloadSharePointFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_download_file",
            "Download SharePoint File",
            "Download a file from SharePoint",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("drive_id", "Drive ID", "Drive ID", VariableType::String);
        node.add_input_pin("item_id", "Item ID", "File item ID", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "content",
            "Content",
            "File content as bytes",
            VariableType::Byte,
        );
        node.add_output_pin(
            "download_url",
            "Download URL",
            "Temporary download URL",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;

        // First get the download URL
        let url = format!(
            "{}/drives/{}/items/{}",
            provider.base_url, drive_id, item_id
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

                if let Some(download_url) = body["@microsoft.graph.downloadUrl"].as_str() {
                    // Download the actual content
                    let content_response = client.get(download_url).send().await;

                    match content_response {
                        Ok(content_resp) if content_resp.status().is_success() => {
                            let bytes = content_resp.bytes().await?;
                            context
                                .set_pin_value("content", json!(bytes.to_vec()))
                                .await?;
                            context
                                .set_pin_value("download_url", json!(download_url))
                                .await?;
                            context.activate_exec_pin("exec_out").await?;
                        }
                        Ok(content_resp) => {
                            let status = content_resp.status();
                            context
                                .set_pin_value(
                                    "error_message",
                                    json!(format!("Download failed: {}", status)),
                                )
                                .await?;
                            context.activate_exec_pin("error").await?;
                        }
                        Err(e) => {
                            context
                                .set_pin_value(
                                    "error_message",
                                    json!(format!("Download request failed: {}", e)),
                                )
                                .await?;
                            context.activate_exec_pin("error").await?;
                        }
                    }
                } else {
                    context
                        .set_pin_value(
                            "error_message",
                            json!("No download URL available for this item"),
                        )
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
// List SharePoint Lists Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListSharePointListsNode {}

impl ListSharePointListsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListSharePointListsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_list_lists",
            "List SharePoint Lists",
            "List all SharePoint lists in a site",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

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
            "site_id",
            "Site ID",
            "SharePoint site ID",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "lists",
            "Lists",
            "List of SharePoint lists",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<SharePointList>();
        node.add_output_pin("count", "Count", "Number of lists", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let site_id: String = context.evaluate_pin("site_id").await?;

        let url = format!("{}/sites/{}/lists", provider.base_url, site_id);

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
                let lists = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_list).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = lists.len() as i64;
                context.set_pin_value("lists", json!(lists)).await?;
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
// Get List Items Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetSharePointListItemsNode {}

impl GetSharePointListItemsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSharePointListItemsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_get_list_items",
            "Get List Items",
            "Get items from a SharePoint list",
            "Data/Microsoft/SharePoint",
        );
        node.add_icon("/flow/icons/sharepoint.svg");

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
            "site_id",
            "Site ID",
            "SharePoint site ID",
            VariableType::String,
        );
        node.add_input_pin(
            "list_id",
            "List ID",
            "SharePoint list ID",
            VariableType::String,
        );
        node.add_input_pin(
            "expand_fields",
            "Expand Fields",
            "Include field values in response",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("items", "Items", "List items", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<SharePointListItem>();
        node.add_output_pin("count", "Count", "Number of items", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let site_id: String = context.evaluate_pin("site_id").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let expand_fields: bool = context.evaluate_pin("expand_fields").await.unwrap_or(true);

        let mut url = format!(
            "{}/sites/{}/lists/{}/items",
            provider.base_url, site_id, list_id
        );
        if expand_fields {
            url.push_str("?expand=fields");
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
                let items = body["value"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_list_item).collect::<Vec<_>>())
                    .unwrap_or_default();

                let count = items.len() as i64;
                context.set_pin_value("items", json!(items)).await?;
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
