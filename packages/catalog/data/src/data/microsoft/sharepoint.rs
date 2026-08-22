use super::{
    graph::{
        flow_path_filename, graph_error_message, graph_get_json, graph_get_paginated_values,
        normalize_graph_path, upload_flow_path_to_drive,
    },
    provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
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
        web_url: item["webUrl"].as_str().unwrap_or_default().to_string(),
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
        web_url: item["webUrl"].as_str().unwrap_or_default().to_string(),
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
        node.set_version(1);
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

        let url = provider.api_url(&format!("/sites?search={}", urlencoding::encode(&query)));

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let sites = values.iter().filter_map(parse_site).collect::<Vec<_>>();
                let count = sites.len() as i64;
                context.set_pin_value("sites", json!(sites)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
        node.set_version(1);
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
        node.add_output_pin(
            "resolved_site_id",
            "Site ID",
            "The site ID",
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
        let hostname: String = context.evaluate_pin("hostname").await.unwrap_or_default();
        let site_path: String = context.evaluate_pin("site_path").await.unwrap_or_default();
        let site_id: String = context.evaluate_pin("site_id").await.unwrap_or_default();

        let url = if !site_id.is_empty() {
            provider.api_url(&format!("/sites/{}", site_id))
        } else if !hostname.is_empty() {
            if site_path.is_empty() {
                provider.api_url(&format!("/sites/{}:", hostname))
            } else {
                provider.api_url(&format!("/sites/{}:{}", hostname, site_path))
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
        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
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
        node.set_version(1);
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

        let url = provider.api_url(&format!("/sites/{}/drives", site_id));

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let drives = values.iter().filter_map(parse_drive).collect::<Vec<_>>();
                let count = drives.len() as i64;
                context.set_pin_value("drives", json!(drives)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
        node.set_version(1);
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
            provider.api_url(&format!(
                "/drives/{}/items/{}/children",
                drive_id, folder_id
            ))
        } else if !folder_path.is_empty() {
            let folder_path = normalize_graph_path(&folder_path);
            provider.api_url(&format!(
                "/drives/{}/root:{}:/children",
                drive_id, folder_path
            ))
        } else {
            provider.api_url(&format!("/drives/{}/root/children", drive_id))
        };

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let items = values
                    .iter()
                    .filter_map(parse_drive_item)
                    .collect::<Vec<_>>();
                let count = items.len() as i64;
                context.set_pin_value("items", json!(items)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
        node.set_version(1);
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
        let url = provider.api_url(&format!("/drives/{}/items/{}", drive_id, item_id));

        let client = reqwest::Client::new();
        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
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
// Get Drive Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetSharePointDriveItemNode {}

impl GetSharePointDriveItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSharePointDriveItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_get_drive_item",
            "Get Drive Item",
            "Get metadata for a SharePoint drive item by ID or path",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin(
            "item_id",
            "Item ID",
            "Drive item ID. Takes precedence over Item Path.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "item_path",
            "Item Path",
            "Path to the drive item when Item ID is empty",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("item", "Item", "Drive item metadata", VariableType::Struct)
            .set_schema::<SharePointDriveItem>();
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
        let item_id: String = context.evaluate_pin("item_id").await.unwrap_or_default();
        let item_path: String = context.evaluate_pin("item_path").await.unwrap_or_default();

        let url = if !item_id.is_empty() {
            provider.api_url(&format!("/drives/{}/items/{}", drive_id, item_id))
        } else if !item_path.is_empty() {
            let item_path = normalize_graph_path(&item_path);
            provider.api_url(&format!("/drives/{}/root:{}", drive_id, item_path))
        } else {
            context
                .set_pin_value(
                    "error_message",
                    json!("Either item_id or item_path is required"),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        };

        let client = reqwest::Client::new();
        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
                if let Some(item) = parse_drive_item(&body) {
                    context.set_pin_value("item", json!(item)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse drive item"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
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
// Upload File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UploadSharePointFileNode {}

impl UploadSharePointFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UploadSharePointFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_upload_file",
            "Upload SharePoint File",
            "Upload a FlowPath file to a SharePoint drive; automatically uses an upload session for larger files",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin(
            "file_path",
            "Destination Path",
            "Destination path including filename. Leave empty to use the FlowPath filename.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "file",
            "File",
            "FlowPath file to upload",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "conflict_behavior",
            "Conflict Behavior",
            "What to do on conflict",
            VariableType::String,
        )
        .set_default_value(Some(json!("rename")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "rename".to_string(),
                    "replace".to_string(),
                    "fail".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "item",
            "Item",
            "Uploaded drive item metadata",
            VariableType::Struct,
        )
        .set_schema::<SharePointDriveItem>();
        node.add_output_pin(
            "used_upload_session",
            "Used Upload Session",
            "True when large-file upload session was used",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "size",
            "Size",
            "Uploaded size in bytes",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let source_file: FlowPath = context.evaluate_pin("file").await?;
        let file_path: String = context.evaluate_pin("file_path").await.unwrap_or_default();
        let conflict_behavior: String = context
            .evaluate_pin("conflict_behavior")
            .await
            .unwrap_or_else(|_| "rename".to_string());

        let destination_path = if file_path.trim().is_empty() {
            flow_path_filename(&source_file)?
        } else {
            file_path
        };
        let destination_path = normalize_graph_path(&destination_path);

        let client = reqwest::Client::new();
        match upload_flow_path_to_drive(
            context,
            &client,
            &provider,
            provider.api_url(&format!(
                "/drives/{}/root:{}:/content",
                drive_id, destination_path
            )),
            provider.api_url(&format!(
                "/drives/{}/root:{}:/createUploadSession",
                drive_id, destination_path
            )),
            &source_file,
            &destination_path,
            &conflict_behavior,
        )
        .await
        {
            Ok(upload) => {
                if let Some(item) = parse_drive_item(&upload.item) {
                    context.set_pin_value("item", json!(item)).await?;
                    context
                        .set_pin_value("used_upload_session", json!(upload.used_upload_session))
                        .await?;
                    context
                        .set_pin_value("size", json!(upload.size as i64))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse upload response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
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
// Create Folder Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateSharePointFolderNode {}

impl CreateSharePointFolderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateSharePointFolderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_create_folder",
            "Create SharePoint Folder",
            "Create a folder in a SharePoint drive",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin(
            "parent_path",
            "Parent Path",
            "Parent folder path when Parent ID is empty",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "parent_id",
            "Parent ID",
            "Parent folder item ID. Takes precedence over Parent Path.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "folder_name",
            "Folder Name",
            "Name of the new folder",
            VariableType::String,
        );
        node.add_input_pin(
            "conflict_behavior",
            "Conflict Behavior",
            "What to do on conflict",
            VariableType::String,
        )
        .set_default_value(Some(json!("rename")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "rename".to_string(),
                    "replace".to_string(),
                    "fail".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "item",
            "Item",
            "Created folder metadata",
            VariableType::Struct,
        )
        .set_schema::<SharePointDriveItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let parent_path: String = context
            .evaluate_pin("parent_path")
            .await
            .unwrap_or_default();
        let parent_id: String = context.evaluate_pin("parent_id").await.unwrap_or_default();
        let folder_name: String = context.evaluate_pin("folder_name").await?;
        let conflict_behavior: String = context
            .evaluate_pin("conflict_behavior")
            .await
            .unwrap_or_else(|_| "rename".to_string());

        let url = if !parent_id.is_empty() {
            provider.api_url(&format!(
                "/drives/{}/items/{}/children",
                drive_id, parent_id
            ))
        } else if parent_path.is_empty() {
            provider.api_url(&format!("/drives/{}/root/children", drive_id))
        } else {
            let parent_path = normalize_graph_path(&parent_path);
            provider.api_url(&format!(
                "/drives/{}/root:{}:/children",
                drive_id, parent_path
            ))
        };

        let body = json!({
            "name": folder_name,
            "folder": {},
            "@microsoft.graph.conflictBehavior": conflict_behavior
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(item) = parse_drive_item(&body) {
                    context.set_pin_value("item", json!(item)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse folder response"))
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Delete Drive Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteSharePointDriveItemNode {}

impl DeleteSharePointDriveItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteSharePointDriveItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_delete_drive_item",
            "Delete Drive Item",
            "Delete a file or folder from a SharePoint drive",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin("item_id", "Item ID", "Drive item ID", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .delete(provider.api_url(&format!("/drives/{}/items/{}", drive_id, item_id)))
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Move Drive Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct MoveSharePointDriveItemNode {}

impl MoveSharePointDriveItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MoveSharePointDriveItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_move_drive_item",
            "Move Drive Item",
            "Move or rename a SharePoint drive item",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
            "Source Drive ID",
            "Source drive ID",
            VariableType::String,
        );
        node.add_input_pin("item_id", "Item ID", "Drive item ID", VariableType::String);
        node.add_input_pin(
            "destination_folder_id",
            "Destination Folder ID",
            "Destination folder item ID",
            VariableType::String,
        );
        node.add_input_pin(
            "destination_drive_id",
            "Destination Drive ID",
            "Optional destination drive ID. Leave empty to use Source Drive ID.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "new_name",
            "New Name",
            "Optional new name for the item",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("item", "Item", "Moved drive item", VariableType::Struct)
            .set_schema::<SharePointDriveItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;
        let destination_folder_id: String = context.evaluate_pin("destination_folder_id").await?;
        let destination_drive_id: String = context
            .evaluate_pin("destination_drive_id")
            .await
            .unwrap_or_default();
        let new_name: String = context.evaluate_pin("new_name").await.unwrap_or_default();

        let mut parent_reference = json!({ "id": destination_folder_id });
        if !destination_drive_id.is_empty() {
            parent_reference["driveId"] = json!(destination_drive_id);
        }

        let mut body = json!({ "parentReference": parent_reference });
        if !new_name.is_empty() {
            body["name"] = json!(new_name);
        }

        let client = reqwest::Client::new();
        let response = client
            .patch(provider.api_url(&format!("/drives/{}/items/{}", drive_id, item_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(item) = parse_drive_item(&body) {
                    context.set_pin_value("item", json!(item)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse move response"))
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Copy Drive Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CopySharePointDriveItemNode {}

impl CopySharePointDriveItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CopySharePointDriveItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_copy_drive_item",
            "Copy Drive Item",
            "Copy a SharePoint drive item asynchronously",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
            "Source Drive ID",
            "Source drive ID",
            VariableType::String,
        );
        node.add_input_pin("item_id", "Item ID", "Drive item ID", VariableType::String);
        node.add_input_pin(
            "destination_folder_id",
            "Destination Folder ID",
            "Destination folder item ID",
            VariableType::String,
        );
        node.add_input_pin(
            "destination_drive_id",
            "Destination Drive ID",
            "Optional destination drive ID. Leave empty to use Source Drive ID.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "new_name",
            "New Name",
            "Optional name for the copy",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "conflict_behavior",
            "Conflict Behavior",
            "What to do on conflict",
            VariableType::String,
        )
        .set_default_value(Some(json!("rename")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "rename".to_string(),
                    "replace".to_string(),
                    "fail".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "children_only",
            "Children Only",
            "Copy only children of a folder",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "include_all_version_history",
            "Include Version History",
            "Copy all version history when supported",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Started", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "monitor_url",
            "Monitor URL",
            "URL to monitor the asynchronous copy operation",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let drive_id: String = context.evaluate_pin("drive_id").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;
        let destination_folder_id: String = context.evaluate_pin("destination_folder_id").await?;
        let destination_drive_id: String = context
            .evaluate_pin("destination_drive_id")
            .await
            .unwrap_or_default();
        let new_name: String = context.evaluate_pin("new_name").await.unwrap_or_default();
        let conflict_behavior: String = context
            .evaluate_pin("conflict_behavior")
            .await
            .unwrap_or_else(|_| "rename".to_string());
        let children_only: bool = context.evaluate_pin("children_only").await.unwrap_or(false);
        let include_all_version_history: bool = context
            .evaluate_pin("include_all_version_history")
            .await
            .unwrap_or(false);

        let mut parent_reference = json!({ "id": destination_folder_id });
        if !destination_drive_id.is_empty() {
            parent_reference["driveId"] = json!(destination_drive_id);
        }

        let mut body = json!({ "parentReference": parent_reference });
        if !new_name.is_empty() {
            body["name"] = json!(new_name);
        }
        if children_only {
            body["childrenOnly"] = json!(true);
        }
        if include_all_version_history {
            body["includeAllVersionHistory"] = json!(true);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url(&format!("/drives/{}/items/{}/copy", drive_id, item_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[(
                "@microsoft.graph.conflictBehavior",
                conflict_behavior.as_str(),
            )])
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
                let monitor_url = resp
                    .headers()
                    .get("Location")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                context
                    .set_pin_value("monitor_url", json!(monitor_url))
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Search Drive Items Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct SearchSharePointDriveItemsNode {}

impl SearchSharePointDriveItemsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchSharePointDriveItemsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_search_drive_items",
            "Search Drive Items",
            "Search files and folders in a SharePoint drive",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin("query", "Query", "Search query", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("items", "Items", "Search results", VariableType::Struct)
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
        let query: String = context.evaluate_pin("query").await?;

        let url = provider.api_url(&format!(
            "/drives/{}/root/search(q='{}')",
            drive_id,
            urlencoding::encode(&query)
        ));

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let items = values
                    .iter()
                    .filter_map(parse_drive_item)
                    .collect::<Vec<_>>();
                let count = items.len() as i64;
                context.set_pin_value("items", json!(items)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
        node.set_version(1);
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

        let url = provider.api_url(&format!("/sites/{}/lists", site_id));

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let lists = values.iter().filter_map(parse_list).collect::<Vec<_>>();
                let count = lists.len() as i64;
                context.set_pin_value("lists", json!(lists)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
        node.set_version(1);
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

        let mut url = provider.api_url(&format!("/sites/{}/lists/{}/items", site_id, list_id));
        if expand_fields {
            url.push_str("?$expand=fields");
        }

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url).await {
            Ok(values) => {
                let items = values
                    .iter()
                    .filter_map(parse_list_item)
                    .collect::<Vec<_>>();
                let count = items.len() as i64;
                context.set_pin_value("items", json!(items)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
// Get List Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetSharePointListItemNode {}

impl GetSharePointListItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSharePointListItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_get_list_item",
            "Get List Item",
            "Get a single SharePoint list item",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin("item_id", "Item ID", "List item ID", VariableType::String);
        node.add_input_pin(
            "expand_fields",
            "Expand Fields",
            "Include field values in response",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("item", "Item", "List item", VariableType::Struct)
            .set_schema::<SharePointListItem>();
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
        let item_id: String = context.evaluate_pin("item_id").await?;
        let expand_fields: bool = context.evaluate_pin("expand_fields").await.unwrap_or(true);

        let mut url = provider.api_url(&format!(
            "/sites/{}/lists/{}/items/{}",
            site_id, list_id, item_id
        ));
        if expand_fields {
            url.push_str("?$expand=fields");
        }

        let client = reqwest::Client::new();
        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
                if let Some(item) = parse_list_item(&body) {
                    context.set_pin_value("item", json!(item)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse list item"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
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
// Create List Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateSharePointListItemNode {}

impl CreateSharePointListItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateSharePointListItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_create_list_item",
            "Create List Item",
            "Create a SharePoint list item from field values",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
            "fields",
            "Fields",
            "Field values keyed by internal field name",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("item", "Item", "Created list item", VariableType::Struct)
            .set_schema::<SharePointListItem>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let site_id: String = context.evaluate_pin("site_id").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let fields: Value = context.evaluate_pin("fields").await?;

        let url = provider.api_url(&format!("/sites/{}/lists/{}/items", site_id, list_id));
        let body = json!({ "fields": fields });

        let client = reqwest::Client::new();
        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(item) = parse_list_item(&body) {
                    context.set_pin_value("item", json!(item)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse created item"))
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Update List Item Fields Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdateSharePointListItemFieldsNode {}

impl UpdateSharePointListItemFieldsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateSharePointListItemFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_update_list_item_fields",
            "Update List Item Fields",
            "Update field values on a SharePoint list item",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin("item_id", "Item ID", "List item ID", VariableType::String);
        node.add_input_pin(
            "fields",
            "Fields",
            "Field values keyed by internal field name",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "updated_fields",
            "Updated Fields",
            "Updated field value set",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let site_id: String = context.evaluate_pin("site_id").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;
        let fields: Value = context.evaluate_pin("fields").await?;

        let url = provider.api_url(&format!(
            "/sites/{}/lists/{}/items/{}/fields",
            site_id, list_id, item_id
        ));

        let client = reqwest::Client::new();
        let response = client
            .patch(url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&fields)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                context.set_pin_value("updated_fields", body).await?;
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Delete List Item Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteSharePointListItemNode {}

impl DeleteSharePointListItemNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteSharePointListItemNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_sharepoint_delete_list_item",
            "Delete List Item",
            "Delete a SharePoint list item",
            "Data/Microsoft/SharePoint",
        );
        node.set_version(1);
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
        node.add_input_pin("item_id", "Item ID", "List item ID", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Sites.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let site_id: String = context.evaluate_pin("site_id").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let item_id: String = context.evaluate_pin("item_id").await?;

        let url = provider.api_url(&format!(
            "/sites/{}/lists/{}/items/{}",
            site_id, list_id, item_id
        ));

        let client = reqwest::Client::new();
        let response = client
            .delete(url)
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
                    .set_pin_value("error_message", json!(format!("Request failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
