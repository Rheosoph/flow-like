use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

use super::provider::{GITHUB_PROVIDER_ID, GitHubProvider};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubWorkflow {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub state: String,
    pub html_url: String,
    pub badge_url: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubWorkflowRun {
    pub id: u64,
    pub name: Option<String>,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub workflow_id: u64,
    pub html_url: String,
    pub run_number: u64,
    pub event: String,
    pub created_at: String,
    pub updated_at: String,
}

fn parse_workflow(workflow: &Value) -> Option<GitHubWorkflow> {
    Some(GitHubWorkflow {
        id: workflow["id"].as_u64()?,
        name: workflow["name"].as_str()?.to_string(),
        path: workflow["path"].as_str()?.to_string(),
        state: workflow["state"].as_str()?.to_string(),
        html_url: workflow["html_url"].as_str()?.to_string(),
        badge_url: workflow["badge_url"].as_str().map(String::from),
        created_at: workflow["created_at"].as_str()?.to_string(),
        updated_at: workflow["updated_at"].as_str()?.to_string(),
    })
}

fn parse_workflow_run(run: &Value) -> Option<GitHubWorkflowRun> {
    Some(GitHubWorkflowRun {
        id: run["id"].as_u64()?,
        name: run["name"].as_str().map(String::from),
        head_branch: run["head_branch"].as_str().map(String::from),
        head_sha: run["head_sha"].as_str()?.to_string(),
        status: run["status"].as_str().map(String::from),
        conclusion: run["conclusion"].as_str().map(String::from),
        workflow_id: run["workflow_id"].as_u64()?,
        html_url: run["html_url"].as_str()?.to_string(),
        run_number: run["run_number"].as_u64()?,
        event: run["event"].as_str()?.to_string(),
        created_at: run["created_at"].as_str()?.to_string(),
        updated_at: run["updated_at"].as_str()?.to_string(),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct ListWorkflowsNode;

impl ListWorkflowsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ListWorkflowsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_list_workflows",
            "List Workflows",
            "List GitHub Actions workflows in a repository",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin(
            "per_page",
            "Per Page",
            "Results per page (max 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_input_pin("page", "Page", "Page number", VariableType::Integer)
            .set_default_value(Some(json!(1)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "workflows",
            "Workflows",
            "List of workflows",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubWorkflow>();
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let per_page: i64 = context.evaluate_pin("per_page").await?;
        let page: i64 = context.evaluate_pin("page").await?;

        let url = format!(
            "{}/repos/{}/{}/actions/workflows?per_page={}&page={}",
            provider.base_url, owner, repo, per_page, page
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let workflows = body["workflows"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_workflow).collect::<Vec<_>>())
                    .unwrap_or_default();

                context.set_pin_value("workflows", json!(workflows)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
pub struct TriggerWorkflowNode;

impl TriggerWorkflowNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for TriggerWorkflowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_trigger_workflow",
            "Trigger Workflow",
            "Trigger a GitHub Actions workflow dispatch event",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin(
            "workflow_id",
            "Workflow ID",
            "Workflow file name or ID (e.g., 'main.yml' or workflow ID)",
            VariableType::String,
        );
        node.add_input_pin(
            "ref",
            "Ref",
            "Git reference (branch or tag) to run the workflow on",
            VariableType::String,
        )
        .set_default_value(Some(json!("main")));
        node.add_input_pin(
            "inputs",
            "Inputs",
            "Workflow inputs as JSON object",
            VariableType::Struct,
        )
        .set_default_value(Some(json!({})));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let workflow_id: String = context.evaluate_pin("workflow_id").await?;
        let git_ref: String = context.evaluate_pin("ref").await?;
        let inputs: Value = context.evaluate_pin("inputs").await?;

        let url = format!(
            "{}/repos/{}/{}/actions/workflows/{}/dispatches",
            provider.base_url, owner, repo, workflow_id
        );

        let body = json!({
            "ref": git_ref,
            "inputs": inputs
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status() == 204 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
pub struct ListWorkflowRunsNode;

impl ListWorkflowRunsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ListWorkflowRunsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_list_workflow_runs",
            "List Workflow Runs",
            "List runs for a specific workflow or all workflows in a repository",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin(
            "workflow_id",
            "Workflow ID",
            "Optional workflow file name or ID to filter runs",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "branch",
            "Branch",
            "Filter by branch name",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("status", "Status", "Filter by status", VariableType::String)
            .set_default_value(Some(json!("")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "".to_string(),
                        "completed".to_string(),
                        "action_required".to_string(),
                        "cancelled".to_string(),
                        "failure".to_string(),
                        "neutral".to_string(),
                        "skipped".to_string(),
                        "stale".to_string(),
                        "success".to_string(),
                        "timed_out".to_string(),
                        "in_progress".to_string(),
                        "queued".to_string(),
                        "requested".to_string(),
                        "waiting".to_string(),
                        "pending".to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "per_page",
            "Per Page",
            "Results per page (max 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_input_pin("page", "Page", "Page number", VariableType::Integer)
            .set_default_value(Some(json!(1)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "runs",
            "Runs",
            "List of workflow runs",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubWorkflowRun>();
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let workflow_id: String = context.evaluate_pin("workflow_id").await?;
        let branch: String = context.evaluate_pin("branch").await?;
        let status: String = context.evaluate_pin("status").await?;
        let per_page: i64 = context.evaluate_pin("per_page").await?;
        let page: i64 = context.evaluate_pin("page").await?;

        let base_url = if workflow_id.is_empty() {
            format!(
                "{}/repos/{}/{}/actions/runs",
                provider.base_url, owner, repo
            )
        } else {
            format!(
                "{}/repos/{}/{}/actions/workflows/{}/runs",
                provider.base_url, owner, repo, workflow_id
            )
        };

        let mut query_params = vec![format!("per_page={}", per_page), format!("page={}", page)];
        if !branch.is_empty() {
            query_params.push(format!("branch={}", branch));
        }
        if !status.is_empty() {
            query_params.push(format!("status={}", status));
        }

        let url = format!("{}?{}", base_url, query_params.join("&"));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let runs = body["workflow_runs"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(parse_workflow_run)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                context.set_pin_value("runs", json!(runs)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
pub struct GetWorkflowRunNode;

impl GetWorkflowRunNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetWorkflowRunNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_get_workflow_run",
            "Get Workflow Run",
            "Get details of a specific workflow run",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin("run_id", "Run ID", "Workflow run ID", VariableType::Integer);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("run", "Run", "Workflow run details", VariableType::Struct)
            .set_schema::<GitHubWorkflowRun>();
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let run_id: i64 = context.evaluate_pin("run_id").await?;

        let url = format!(
            "{}/repos/{}/{}/actions/runs/{}",
            provider.base_url, owner, repo, run_id
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(run) = parse_workflow_run(&body) {
                    context.set_pin_value("run", json!(run)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse workflow run"))
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
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
pub struct CancelWorkflowRunNode;

impl CancelWorkflowRunNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CancelWorkflowRunNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_cancel_workflow_run",
            "Cancel Workflow Run",
            "Cancel a workflow run that is in progress",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin(
            "run_id",
            "Run ID",
            "Workflow run ID to cancel",
            VariableType::Integer,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let run_id: i64 = context.evaluate_pin("run_id").await?;

        let url = format!(
            "{}/repos/{}/{}/actions/runs/{}/cancel",
            provider.base_url, owner, repo, run_id
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status() == 202 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
pub struct RerunWorkflowNode;

impl RerunWorkflowNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RerunWorkflowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_rerun_workflow",
            "Rerun Workflow",
            "Re-run a workflow run",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin(
            "run_id",
            "Run ID",
            "Workflow run ID to rerun",
            VariableType::Integer,
        );
        node.add_input_pin(
            "failed_only",
            "Failed Only",
            "Only rerun failed jobs",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let run_id: i64 = context.evaluate_pin("run_id").await?;
        let failed_only: bool = context.evaluate_pin("failed_only").await?;

        let endpoint = if failed_only {
            "rerun-failed-jobs"
        } else {
            "rerun"
        };

        let url = format!(
            "{}/repos/{}/{}/actions/runs/{}/{}",
            provider.base_url, owner, repo, run_id, endpoint
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status() == 201 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
pub struct GetLatestWorkflowRunNode;

impl GetLatestWorkflowRunNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetLatestWorkflowRunNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "github_get_latest_workflow_run",
            "Get Latest Workflow Run",
            "Get the most recent workflow run, optionally filtered by conclusion (success/failure)",
            "Data/GitHub/Workflows",
        );
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider with authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin("repo", "Repo", "Repository name", VariableType::String);
        node.add_input_pin(
            "workflow_id",
            "Workflow ID",
            "Workflow file name or ID (e.g., 'ci.yml')",
            VariableType::String,
        );
        node.add_input_pin(
            "branch",
            "Branch",
            "Filter by branch name (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "conclusion",
            "Conclusion",
            "Filter by conclusion (empty = any, success, failure, cancelled, skipped)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "".to_string(),
                    "success".to_string(),
                    "failure".to_string(),
                    "cancelled".to_string(),
                    "skipped".to_string(),
                    "timed_out".to_string(),
                    "action_required".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "exclude_pull_requests",
            "Exclude PRs",
            "Exclude runs triggered by pull requests",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "not_found",
            "Not Found",
            "No matching run found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "run",
            "Run",
            "The latest workflow run",
            VariableType::Struct,
        )
        .set_schema::<GitHubWorkflowRun>();
        node.add_output_pin(
            "run_id",
            "Run ID",
            "The workflow run ID",
            VariableType::Integer,
        );
        node.add_output_pin(
            "run_number",
            "Run Number",
            "The workflow run number",
            VariableType::Integer,
        );
        node.add_output_pin(
            "status",
            "Status",
            "Run status (queued, in_progress, completed)",
            VariableType::String,
        );
        node.add_output_pin(
            "run_conclusion",
            "Conclusion",
            "Run conclusion (success, failure, etc.)",
            VariableType::String,
        );
        node.add_output_pin(
            "head_branch",
            "Branch",
            "The branch the run was triggered on",
            VariableType::String,
        );
        node.add_output_pin(
            "head_sha",
            "Commit SHA",
            "The commit SHA",
            VariableType::String,
        );
        node.add_output_pin(
            "html_url",
            "URL",
            "Link to the workflow run",
            VariableType::String,
        );
        node.add_output_pin(
            "is_success",
            "Is Success",
            "Whether the run completed successfully",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if request failed",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;
        context.deactivate_exec_pin("not_found").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let workflow_id: String = context.evaluate_pin("workflow_id").await?;
        let branch: String = context.evaluate_pin("branch").await?;
        let conclusion: String = context.evaluate_pin("conclusion").await?;
        let exclude_prs: bool = context.evaluate_pin("exclude_pull_requests").await?;

        let base_url = if workflow_id.is_empty() {
            format!(
                "{}/repos/{}/{}/actions/runs",
                provider.base_url, owner, repo
            )
        } else {
            format!(
                "{}/repos/{}/{}/actions/workflows/{}/runs",
                provider.base_url, owner, repo, workflow_id
            )
        };

        let mut query_params = vec!["per_page=10".to_string(), "page=1".to_string()];
        if !branch.is_empty() {
            query_params.push(format!("branch={}", branch));
        }
        if !conclusion.is_empty() {
            query_params.push("status=completed".to_string());
        }
        if exclude_prs {
            query_params.push("exclude_pull_requests=true".to_string());
        }

        let url = format!("{}?{}", base_url, query_params.join("&"));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let runs = body["workflow_runs"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(parse_workflow_run)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                // Filter by conclusion if specified (GitHub API doesn't support conclusion filter directly)
                let filtered_runs: Vec<_> = if conclusion.is_empty() {
                    runs
                } else {
                    runs.into_iter()
                        .filter(|r| r.conclusion.as_deref() == Some(conclusion.as_str()))
                        .collect()
                };

                if let Some(latest) = filtered_runs.first() {
                    let is_success = latest.conclusion.as_deref() == Some("success");

                    context.set_pin_value("run", json!(latest)).await?;
                    context.set_pin_value("run_id", json!(latest.id)).await?;
                    context
                        .set_pin_value("run_number", json!(latest.run_number))
                        .await?;
                    context
                        .set_pin_value("status", json!(latest.status.clone().unwrap_or_default()))
                        .await?;
                    context
                        .set_pin_value(
                            "run_conclusion",
                            json!(latest.conclusion.clone().unwrap_or_default()),
                        )
                        .await?;
                    context
                        .set_pin_value(
                            "head_branch",
                            json!(latest.head_branch.clone().unwrap_or_default()),
                        )
                        .await?;
                    context
                        .set_pin_value("head_sha", json!(latest.head_sha.clone()))
                        .await?;
                    context
                        .set_pin_value("html_url", json!(latest.html_url.clone()))
                        .await?;
                    context
                        .set_pin_value("is_success", json!(is_success))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.activate_exec_pin("not_found").await?;
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_default();
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("GitHub API error {}: {}", status, error_text)),
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
