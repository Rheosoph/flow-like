use super::provider::{DATABRICKS_PROVIDER_ID, DatabricksProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksCluster {
    pub cluster_id: String,
    pub cluster_name: String,
    pub state: String,
    pub state_message: Option<String>,
    pub spark_version: String,
    pub node_type_id: String,
    pub driver_node_type_id: Option<String>,
    pub num_workers: Option<i64>,
    pub autoscale_min_workers: Option<i64>,
    pub autoscale_max_workers: Option<i64>,
    pub creator_user_name: Option<String>,
    pub start_time: Option<i64>,
    pub terminated_time: Option<i64>,
    pub cluster_source: Option<String>,
}

fn parse_cluster(cluster: &Value) -> Option<DatabricksCluster> {
    let autoscale = cluster.get("autoscale");

    Some(DatabricksCluster {
        cluster_id: cluster["cluster_id"].as_str()?.to_string(),
        cluster_name: cluster["cluster_name"].as_str()?.to_string(),
        state: cluster["state"].as_str().unwrap_or("UNKNOWN").to_string(),
        state_message: cluster["state_message"].as_str().map(String::from),
        spark_version: cluster["spark_version"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        node_type_id: cluster["node_type_id"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        driver_node_type_id: cluster["driver_node_type_id"].as_str().map(String::from),
        num_workers: cluster["num_workers"].as_i64(),
        autoscale_min_workers: autoscale.and_then(|a| a["min_workers"].as_i64()),
        autoscale_max_workers: autoscale.and_then(|a| a["max_workers"].as_i64()),
        creator_user_name: cluster["creator_user_name"].as_str().map(String::from),
        start_time: cluster["start_time"].as_i64(),
        terminated_time: cluster["terminated_time"].as_i64(),
        cluster_source: cluster["cluster_source"].as_str().map(String::from),
    })
}

// =============================================================================
// List Clusters Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksClustersNode {}

impl ListDatabricksClustersNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksClustersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_clusters",
            "List Clusters",
            "List all clusters in the Databricks workspace",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "listClusters");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the cluster listing",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin(
            "clusters",
            "Clusters",
            "Array of clusters",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<DatabricksCluster>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of clusters returned",
            VariableType::Integer,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;

        let url = provider.api_url("/clusters/list");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let clusters_array = data["clusters"].as_array();
                    let clusters: Vec<DatabricksCluster> = clusters_array
                        .map(|arr| arr.iter().filter_map(parse_cluster).collect())
                        .unwrap_or_default();

                    let count = clusters.len();

                    context.set_pin_value("clusters", json!(clusters)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
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
// Get Cluster Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetDatabricksClusterNode {}

impl GetDatabricksClusterNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetDatabricksClusterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_get_cluster",
            "Get Cluster",
            "Get details of a specific cluster by ID",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "getCluster");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "cluster_id",
            "Cluster ID",
            "The ID of the cluster to retrieve",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin(
            "cluster",
            "Cluster",
            "Cluster details",
            VariableType::Struct,
        )
        .set_schema::<DatabricksCluster>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let cluster_id: String = context.evaluate_pin("cluster_id").await?;

        if cluster_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Cluster ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url("/clusters/get");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("cluster_id", &cluster_id)])
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    if let Some(cluster) = parse_cluster(&data) {
                        context.set_pin_value("cluster", json!(cluster)).await?;
                        context.set_pin_value("error_message", json!("")).await?;
                        context.activate_exec_pin("exec_out").await?;
                    } else {
                        context
                            .set_pin_value("error_message", json!("Failed to parse cluster data"))
                            .await?;
                        context.activate_exec_pin("error").await?;
                    }
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
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
// Start Cluster Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct StartDatabricksClusterNode {}

impl StartDatabricksClusterNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StartDatabricksClusterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_start_cluster",
            "Start Cluster",
            "Start a terminated cluster",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "startCluster");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "cluster_id",
            "Cluster ID",
            "The ID of the cluster to start",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when start request is accepted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let cluster_id: String = context.evaluate_pin("cluster_id").await?;

        if cluster_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Cluster ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url("/clusters/start");

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&json!({ "cluster_id": cluster_id }))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
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
// Stop Cluster Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct StopDatabricksClusterNode {}

impl StopDatabricksClusterNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StopDatabricksClusterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_stop_cluster",
            "Stop Cluster",
            "Terminate a running cluster",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "stopCluster");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "cluster_id",
            "Cluster ID",
            "The ID of the cluster to terminate",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when terminate request is accepted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let cluster_id: String = context.evaluate_pin("cluster_id").await?;

        if cluster_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Cluster ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url("/clusters/delete");

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&json!({ "cluster_id": cluster_id }))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
