use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

/// Jira project version (release)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraVersion {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
    pub released: bool,
    pub release_date: Option<String>,
    pub start_date: Option<String>,
    pub project_id: i64,
}

fn parse_version(value: &Value) -> Option<JiraVersion> {
    let obj = value.as_object()?;

    Some(JiraVersion {
        id: obj.get("id")?.as_str()?.to_string(),
        name: obj.get("name")?.as_str()?.to_string(),
        description: obj
            .get("description")
            .and_then(|d| d.as_str())
            .map(String::from),
        archived: obj
            .get("archived")
            .and_then(|a| a.as_bool())
            .unwrap_or(false),
        released: obj
            .get("released")
            .and_then(|r| r.as_bool())
            .unwrap_or(false),
        release_date: obj
            .get("releaseDate")
            .and_then(|d| d.as_str())
            .map(String::from),
        start_date: obj
            .get("startDate")
            .and_then(|d| d.as_str())
            .map(String::from),
        project_id: obj.get("projectId").and_then(|p| p.as_i64()).unwrap_or(0),
    })
}

/// Get versions for a project
#[crate::register_node]
#[derive(Default)]
pub struct GetVersionsNode {}

impl GetVersionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetVersionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_versions",
            "Get Versions",
            "Get all versions (releases) for a project",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getVersions");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "project_key",
            "Project Key",
            "The project key (e.g., PROJ)",
            VariableType::String,
        );

        node.add_output_pin(
            "versions",
            "Versions",
            "List of versions",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraVersion>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of versions",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let project_key: String = context.evaluate_pin("project_key").await?;

        if project_key.is_empty() {
            return Err(flow_like_types::anyhow!("Project key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/project/{}/versions", project_key));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get versions: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let versions: Vec<JiraVersion> = data
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_version)
            .collect();

        let count = versions.len() as i64;

        context.set_pin_value("versions", json!(versions)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Create a new version
#[crate::register_node]
#[derive(Default)]
pub struct CreateVersionNode {}

impl CreateVersionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateVersionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_create_version",
            "Create Version",
            "Create a new version (release) in a project",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "createVersion");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("name", "Name", "Name of the version", VariableType::String);

        node.add_input_pin(
            "project_key",
            "Project Key",
            "The project key (e.g., PROJ)",
            VariableType::String,
        );

        node.add_input_pin(
            "description",
            "Description",
            "Description of the version (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "release_date",
            "Release Date",
            "Planned release date (YYYY-MM-DD, optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "start_date",
            "Start Date",
            "Start date (YYYY-MM-DD, optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "released",
            "Released",
            "Whether the version is already released (default: false)",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "version",
            "Version",
            "The created version",
            VariableType::Struct,
        )
        .set_schema::<JiraVersion>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let name: String = context.evaluate_pin("name").await?;
        let project_key: String = context.evaluate_pin("project_key").await?;
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let release_date: String = context
            .evaluate_pin("release_date")
            .await
            .unwrap_or_default();
        let start_date: String = context.evaluate_pin("start_date").await.unwrap_or_default();
        let released: bool = context.evaluate_pin("released").await.unwrap_or(false);

        if name.is_empty() {
            return Err(flow_like_types::anyhow!("Version name is required"));
        }
        if project_key.is_empty() {
            return Err(flow_like_types::anyhow!("Project key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/version");

        let mut body = json!({
            "name": name,
            "project": project_key,
            "released": released
        });

        if !description.is_empty() {
            body["description"] = json!(description);
        }
        if !release_date.is_empty() {
            body["releaseDate"] = json!(release_date);
        }
        if !start_date.is_empty() {
            body["startDate"] = json!(start_date);
        }

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to create version: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let version = parse_version(&data);

        context.set_pin_value("version", json!(version)).await?;

        Ok(())
    }
}

/// Input structure for batch version creation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchVersionInput {
    pub project_key: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub released: Option<bool>,
    #[serde(default)]
    pub archived: Option<bool>,
}

/// Result of a batch version creation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchVersionResult {
    pub success: bool,
    pub version_id: Option<String>,
    pub version_name: Option<String>,
    pub error: Option<String>,
    pub input_index: usize,
}

/// Batch create versions
#[crate::register_node]
#[derive(Default)]
pub struct BatchCreateVersionsNode {}

impl BatchCreateVersionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BatchCreateVersionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_batch_create_versions",
            "Batch Create Versions",
            "Create multiple versions (releases) in a batch",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "batchCreateVersions");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "versions",
            "Versions",
            "Array of versions to create",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<BatchVersionInput>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "results",
            "Results",
            "Results for each version creation",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<BatchVersionResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "created_count",
            "Created Count",
            "Number of successfully created versions",
            VariableType::Integer,
        );

        node.add_output_pin(
            "failed_count",
            "Failed Count",
            "Number of failed version creations",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let versions: Vec<BatchVersionInput> = context.evaluate_pin("versions").await?;

        if versions.is_empty() {
            context.set_pin_value("results", json!([])).await?;
            context.set_pin_value("created_count", json!(0)).await?;
            context.set_pin_value("failed_count", json!(0)).await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/version");
        let mut results: Vec<BatchVersionResult> = Vec::with_capacity(versions.len());
        let mut created_count = 0i64;
        let mut failed_count = 0i64;

        for (index, version_input) in versions.iter().enumerate() {
            let mut body = json!({
                "project": version_input.project_key,
                "name": version_input.name,
            });

            if let Some(ref desc) = version_input.description {
                body["description"] = json!(desc);
            }
            if let Some(ref date) = version_input.release_date {
                body["releaseDate"] = json!(date);
            }
            if let Some(ref date) = version_input.start_date {
                body["startDate"] = json!(date);
            }
            if let Some(released) = version_input.released {
                body["released"] = json!(released);
            }
            if let Some(archived) = version_input.archived {
                body["archived"] = json!(archived);
            }

            let response = client
                .post(&url)
                .header("Authorization", provider.auth_header())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(data) = resp.json::<Value>().await {
                        let version_id = data.get("id").and_then(|v| v.as_str()).map(String::from);
                        let version_name =
                            data.get("name").and_then(|v| v.as_str()).map(String::from);
                        results.push(BatchVersionResult {
                            success: true,
                            version_id,
                            version_name,
                            error: None,
                            input_index: index,
                        });
                        created_count += 1;
                    } else {
                        results.push(BatchVersionResult {
                            success: false,
                            version_id: None,
                            version_name: None,
                            error: Some("Failed to parse response".to_string()),
                            input_index: index,
                        });
                        failed_count += 1;
                    }
                }
                Ok(resp) => {
                    let error_text = resp.text().await.unwrap_or_default();
                    results.push(BatchVersionResult {
                        success: false,
                        version_id: None,
                        version_name: None,
                        error: Some(error_text),
                        input_index: index,
                    });
                    failed_count += 1;
                }
                Err(e) => {
                    results.push(BatchVersionResult {
                        success: false,
                        version_id: None,
                        version_name: None,
                        error: Some(e.to_string()),
                        input_index: index,
                    });
                    failed_count += 1;
                }
            }
        }

        context.set_pin_value("results", json!(results)).await?;
        context
            .set_pin_value("created_count", json!(created_count))
            .await?;
        context
            .set_pin_value("failed_count", json!(failed_count))
            .await?;

        Ok(())
    }
}
/// Update an existing version
#[crate::register_node]
#[derive(Default)]
pub struct UpdateVersionNode {}

impl UpdateVersionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateVersionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_update_version",
            "Update Version",
            "Update an existing version",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "updateVersion");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "version_id",
            "Version ID",
            "The version ID to update",
            VariableType::String,
        );

        node.add_input_pin(
            "name",
            "Name",
            "New name for the version (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "description",
            "Description",
            "New description (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "released",
            "Released",
            "Set released status (optional)",
            VariableType::Boolean,
        );

        node.add_input_pin(
            "archived",
            "Archived",
            "Set archived status (optional)",
            VariableType::Boolean,
        );

        node.add_input_pin(
            "release_date",
            "Release Date",
            "New release date (YYYY-MM-DD, optional)",
            VariableType::String,
        );

        node.add_output_pin(
            "version",
            "Version",
            "The updated version",
            VariableType::Struct,
        )
        .set_schema::<JiraVersion>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let version_id: String = context.evaluate_pin("version_id").await?;
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let release_date: String = context
            .evaluate_pin("release_date")
            .await
            .unwrap_or_default();

        if version_id.is_empty() {
            return Err(flow_like_types::anyhow!("Version ID is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/version/{}", version_id));

        let mut body = json!({});

        if !name.is_empty() {
            body["name"] = json!(name);
        }
        if !description.is_empty() {
            body["description"] = json!(description);
        }
        if !release_date.is_empty() {
            body["releaseDate"] = json!(release_date);
        }

        // Handle optional booleans
        if let Ok(released) = context.evaluate_pin::<bool>("released").await {
            body["released"] = json!(released);
        }
        if let Ok(archived) = context.evaluate_pin::<bool>("archived").await {
            body["archived"] = json!(archived);
        }

        let response = client
            .put(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to update version: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let version = parse_version(&data);

        context.set_pin_value("version", json!(version)).await?;

        Ok(())
    }
}
