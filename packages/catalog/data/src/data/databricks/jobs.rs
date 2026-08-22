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
pub struct DatabricksJob {
    pub job_id: i64,
    pub creator_user_name: Option<String>,
    pub name: String,
    pub settings: DatabricksJobSettings,
    pub created_time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksJobSettings {
    pub name: Option<String>,
    pub max_concurrent_runs: Option<i64>,
    pub timeout_seconds: Option<i64>,
    pub email_notifications: Option<Value>,
    pub schedule: Option<DatabricksJobSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksJobSchedule {
    pub quartz_cron_expression: String,
    pub timezone_id: String,
    pub pause_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksJobRun {
    pub run_id: i64,
    pub job_id: i64,
    pub run_name: Option<String>,
    pub state: DatabricksRunState,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub run_duration: Option<i64>,
    pub run_type: String,
    pub trigger: Option<String>,
    pub creator_user_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksRunState {
    pub life_cycle_state: String,
    pub result_state: Option<String>,
    pub state_message: Option<String>,
}

fn parse_job(job: &Value) -> Option<DatabricksJob> {
    let settings = &job["settings"];
    Some(DatabricksJob {
        job_id: job["job_id"].as_i64()?,
        creator_user_name: job["creator_user_name"].as_str().map(String::from),
        name: settings["name"].as_str().unwrap_or_default().to_string(),
        settings: DatabricksJobSettings {
            name: settings["name"].as_str().map(String::from),
            max_concurrent_runs: settings["max_concurrent_runs"].as_i64(),
            timeout_seconds: settings["timeout_seconds"].as_i64(),
            email_notifications: settings.get("email_notifications").cloned(),
            schedule: settings.get("schedule").and_then(|s| {
                Some(DatabricksJobSchedule {
                    quartz_cron_expression: s["quartz_cron_expression"].as_str()?.to_string(),
                    timezone_id: s["timezone_id"].as_str()?.to_string(),
                    pause_status: s["pause_status"].as_str().map(String::from),
                })
            }),
        },
        created_time: job["created_time"].as_i64().unwrap_or(0),
    })
}

fn parse_job_run(run: &Value) -> Option<DatabricksJobRun> {
    let state = &run["state"];
    Some(DatabricksJobRun {
        run_id: run["run_id"].as_i64()?,
        job_id: run["job_id"].as_i64().unwrap_or(0),
        run_name: run["run_name"].as_str().map(String::from),
        state: DatabricksRunState {
            life_cycle_state: state["life_cycle_state"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .to_string(),
            result_state: state["result_state"].as_str().map(String::from),
            state_message: state["state_message"].as_str().map(String::from),
        },
        start_time: run["start_time"].as_i64().unwrap_or(0),
        end_time: run["end_time"].as_i64(),
        run_duration: run["run_duration"].as_i64(),
        run_type: run["run_type"].as_str().unwrap_or_default().to_string(),
        trigger: run["trigger"].as_str().map(String::from),
        creator_user_name: run["creator_user_name"].as_str().map(String::from),
    })
}

// =============================================================================
// List Jobs Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksJobsNode {}

impl ListDatabricksJobsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksJobsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_jobs",
            "List Jobs",
            "List all jobs in the Databricks workspace",
            "Data/Databricks",
        );
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
            "limit",
            "Limit",
            "Maximum number of jobs to return (default: 25, max: 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(25)));

        node.add_input_pin(
            "offset",
            "Offset",
            "Offset for pagination",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "name",
            "Name Filter",
            "Optional: Filter jobs by name (substring match)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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

        node.add_output_pin("jobs", "Jobs", "Array of jobs", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<DatabricksJob>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of jobs returned",
            VariableType::Integer,
        );

        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more jobs available",
            VariableType::Boolean,
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
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(25);
        let offset: i64 = context.evaluate_pin("offset").await.unwrap_or(0);
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();

        let mut url = provider.api_url_v21(&format!(
            "/jobs/list?limit={}&offset={}",
            limit.clamp(1, 100),
            offset.max(0)
        ));

        if !name.is_empty() {
            url.push_str(&format!("&name={}", urlencoding::encode(&name)));
        }

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

                    let jobs_array = data["jobs"].as_array();
                    let jobs: Vec<DatabricksJob> = jobs_array
                        .map(|arr| arr.iter().filter_map(parse_job).collect())
                        .unwrap_or_default();

                    let count = jobs.len();
                    let has_more = data["has_more"].as_bool().unwrap_or(false);

                    context.set_pin_value("jobs", json!(jobs)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("has_more", json!(has_more)).await?;
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
// Run Job Now Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct RunDatabricksJobNode {}

impl RunDatabricksJobNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RunDatabricksJobNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_run_job",
            "Run Job",
            "Trigger a job run immediately",
            "Data/Databricks",
        );
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
            "job_id",
            "Job ID",
            "The ID of the job to run",
            VariableType::Integer,
        );

        node.add_input_pin(
            "job_parameters",
            "Job Parameters",
            "Optional: JSON object with job parameters",
            VariableType::Struct,
        )
        .set_default_value(Some(json!({})))
        .set_open_schema();

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when job is started",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin(
            "run_id",
            "Run ID",
            "The ID of the job run",
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
                .set_security(7)
                .set_performance(6)
                .set_governance(6)
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
        let job_id: i64 = context.evaluate_pin("job_id").await?;
        let job_parameters: Value = context
            .evaluate_pin("job_parameters")
            .await
            .unwrap_or(json!({}));

        if job_id == 0 {
            context
                .set_pin_value("error_message", json!("Job ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url_v21("/jobs/run-now");

        let mut body = json!({ "job_id": job_id });
        if let Some(params) = job_parameters.as_object()
            && !params.is_empty()
        {
            body["job_parameters"] = job_parameters;
        }

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let run_id = data["run_id"].as_i64().unwrap_or(0);

                    context.set_pin_value("run_id", json!(run_id)).await?;
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
// Get Job Run Status Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetDatabricksJobRunNode {}

impl GetDatabricksJobRunNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetDatabricksJobRunNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_get_job_run",
            "Get Job Run",
            "Get the status of a job run",
            "Data/Databricks",
        );
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
            "run_id",
            "Run ID",
            "The ID of the job run",
            VariableType::Integer,
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

        node.add_output_pin("run", "Run", "Job run details", VariableType::Struct)
            .set_schema::<DatabricksJobRun>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "is_running",
            "Is Running",
            "Whether the job is still running",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "is_successful",
            "Is Successful",
            "Whether the job completed successfully",
            VariableType::Boolean,
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
        let run_id: i64 = context.evaluate_pin("run_id").await?;

        if run_id == 0 {
            context
                .set_pin_value("error_message", json!("Run ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url_v21("/jobs/runs/get");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("run_id", run_id.to_string())])
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    if let Some(run) = parse_job_run(&data) {
                        let is_running = matches!(
                            run.state.life_cycle_state.as_str(),
                            "PENDING" | "RUNNING" | "TERMINATING"
                        );
                        let is_successful = run.state.result_state.as_deref() == Some("SUCCESS");

                        context.set_pin_value("run", json!(run)).await?;
                        context
                            .set_pin_value("is_running", json!(is_running))
                            .await?;
                        context
                            .set_pin_value("is_successful", json!(is_successful))
                            .await?;
                        context.set_pin_value("error_message", json!("")).await?;
                        context.activate_exec_pin("exec_out").await?;
                    } else {
                        context
                            .set_pin_value("error_message", json!("Failed to parse run data"))
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
// Cancel Job Run Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CancelDatabricksJobRunNode {}

impl CancelDatabricksJobRunNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CancelDatabricksJobRunNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_cancel_job_run",
            "Cancel Job Run",
            "Cancel a running job",
            "Data/Databricks",
        );
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
            "run_id",
            "Run ID",
            "The ID of the job run to cancel",
            VariableType::Integer,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when cancel is requested",
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
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let run_id: i64 = context.evaluate_pin("run_id").await?;

        if run_id == 0 {
            context
                .set_pin_value("error_message", json!("Run ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url_v21("/jobs/runs/cancel");

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&json!({ "run_id": run_id }))
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
