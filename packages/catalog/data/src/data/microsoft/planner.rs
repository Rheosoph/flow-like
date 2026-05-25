use super::{
    graph::{graph_error_message, graph_get_json, graph_get_paginated_values},
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
// Planner Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlannerPlan {
    pub id: String,
    pub title: String,
    pub owner: Option<String>,
    pub created_date_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlannerTask {
    pub id: String,
    pub title: String,
    pub plan_id: String,
    pub bucket_id: Option<String>,
    pub percent_complete: i64,
    pub priority: Option<i64>,
    pub start_date_time: Option<String>,
    pub due_date_time: Option<String>,
    pub created_date_time: Option<String>,
    pub completed_date_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlannerBucket {
    pub id: String,
    pub name: String,
    pub plan_id: String,
    pub order_hint: String,
}

fn parse_plan(value: &Value) -> Option<PlannerPlan> {
    Some(PlannerPlan {
        id: value["id"].as_str()?.to_string(),
        title: value["title"].as_str()?.to_string(),
        owner: value["owner"].as_str().map(String::from),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
    })
}

fn parse_task(value: &Value) -> Option<PlannerTask> {
    Some(PlannerTask {
        id: value["id"].as_str()?.to_string(),
        title: value["title"].as_str()?.to_string(),
        plan_id: value["planId"].as_str()?.to_string(),
        bucket_id: value["bucketId"].as_str().map(String::from),
        percent_complete: value["percentComplete"].as_i64().unwrap_or(0),
        priority: value["priority"].as_i64(),
        start_date_time: value["startDateTime"].as_str().map(String::from),
        due_date_time: value["dueDateTime"].as_str().map(String::from),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        completed_date_time: value["completedDateTime"].as_str().map(String::from),
    })
}

fn parse_bucket(value: &Value) -> Option<PlannerBucket> {
    Some(PlannerBucket {
        id: value["id"].as_str()?.to_string(),
        name: value["name"].as_str()?.to_string(),
        plan_id: value["planId"].as_str()?.to_string(),
        order_hint: value["orderHint"].as_str().unwrap_or("").to_string(),
    })
}

// =============================================================================
// List My Plans Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListMyPlansNode {}

impl ListMyPlansNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListMyPlansNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_list_my_plans",
            "List My Plans",
            "List all Planner plans the user has access to",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

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
        node.add_output_pin("plans", "Plans", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<PlannerPlan>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.Read", "Group.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, provider.api_url("/me/planner/plans"))
            .await
        {
            Ok(values) => {
                let plans: Vec<PlannerPlan> = values.iter().filter_map(parse_plan).collect();
                let count = plans.len() as i64;
                context.set_pin_value("plans", json!(plans)).await?;
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
// Get Plan Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetPlanNode {}

impl GetPlanNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetPlanNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_get_plan",
            "Get Plan",
            "Get details of a specific Planner plan",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("plan_id", "Plan ID", "ID of the plan", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("plan", "Plan", "", VariableType::Struct)
            .set_schema::<PlannerPlan>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let plan_id: String = context.evaluate_pin("plan_id").await?;

        let client = reqwest::Client::new();
        match graph_get_json(
            &client,
            &provider,
            provider.api_url(&format!("/planner/plans/{}", plan_id)),
        )
        .await
        {
            Ok(body) => {
                if let Some(plan) = parse_plan(&body) {
                    context.set_pin_value("plan", json!(plan)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
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
// List Plan Tasks Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListPlanTasksNode {}

impl ListPlanTasksNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListPlanTasksNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_list_tasks",
            "List Plan Tasks",
            "List all tasks in a Planner plan",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("plan_id", "Plan ID", "ID of the plan", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("tasks", "Tasks", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<PlannerTask>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let plan_id: String = context.evaluate_pin("plan_id").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&format!("/planner/plans/{}/tasks", plan_id)),
        )
        .await
        {
            Ok(values) => {
                let tasks: Vec<PlannerTask> = values.iter().filter_map(parse_task).collect();
                let count = tasks.len() as i64;
                context.set_pin_value("tasks", json!(tasks)).await?;
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
// List Plan Buckets Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListPlanBucketsNode {}

impl ListPlanBucketsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListPlanBucketsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_list_buckets",
            "List Plan Buckets",
            "List all buckets in a Planner plan",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("plan_id", "Plan ID", "ID of the plan", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("buckets", "Buckets", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<PlannerBucket>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let plan_id: String = context.evaluate_pin("plan_id").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&format!("/planner/plans/{}/buckets", plan_id)),
        )
        .await
        {
            Ok(values) => {
                let buckets: Vec<PlannerBucket> = values.iter().filter_map(parse_bucket).collect();
                let count = buckets.len() as i64;
                context.set_pin_value("buckets", json!(buckets)).await?;
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
// Create Planner Task Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreatePlannerTaskNode {}

impl CreatePlannerTaskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreatePlannerTaskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_create_task",
            "Create Planner Task",
            "Create a new task in a Planner plan",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("plan_id", "Plan ID", "ID of the plan", VariableType::String);
        node.add_input_pin("title", "Title", "Task title", VariableType::String);
        node.add_input_pin(
            "bucket_id",
            "Bucket ID",
            "ID of the bucket (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "due_date",
            "Due Date",
            "Due date (ISO format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "priority",
            "Priority",
            "Task priority (1=urgent, 3=important, 5=medium, 9=low)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("task", "Task", "", VariableType::Struct)
            .set_schema::<PlannerTask>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let plan_id: String = context.evaluate_pin("plan_id").await?;
        let title: String = context.evaluate_pin("title").await?;
        let bucket_id: String = context.evaluate_pin("bucket_id").await.unwrap_or_default();
        let due_date: String = context.evaluate_pin("due_date").await.unwrap_or_default();
        let priority: i64 = context.evaluate_pin("priority").await.unwrap_or(5);

        let mut request_body = json!({
            "planId": plan_id,
            "title": title,
            "priority": priority
        });

        if !bucket_id.is_empty() {
            request_body["bucketId"] = json!(bucket_id);
        }
        if !due_date.is_empty() {
            request_body["dueDateTime"] = json!(due_date);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/planner/tasks"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(task) = parse_task(&body) {
                    context.set_pin_value("task", json!(task)).await?;
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
// Update Planner Task Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdatePlannerTaskNode {}

impl UpdatePlannerTaskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdatePlannerTaskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_update_task",
            "Update Planner Task",
            "Update an existing Planner task",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("task_id", "Task ID", "ID of the task", VariableType::String);
        node.add_input_pin(
            "etag",
            "ETag",
            "Current ETag of the task",
            VariableType::String,
        );
        node.add_input_pin(
            "title",
            "Title",
            "New task title (leave empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "percent_complete",
            "Percent Complete",
            "Completion percentage (0-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        node.add_input_pin(
            "priority",
            "Priority",
            "Task priority (-1 to keep)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let task_id: String = context.evaluate_pin("task_id").await?;
        let etag: String = context.evaluate_pin("etag").await?;
        let title: String = context.evaluate_pin("title").await.unwrap_or_default();
        let percent_complete: i64 = context.evaluate_pin("percent_complete").await.unwrap_or(-1);
        let priority: i64 = context.evaluate_pin("priority").await.unwrap_or(-1);

        let mut request_body = json!({});

        if !title.is_empty() {
            request_body["title"] = json!(title);
        }
        if percent_complete >= 0 {
            request_body["percentComplete"] = json!(percent_complete);
        }
        if priority >= 0 {
            request_body["priority"] = json!(priority);
        }

        let client = reqwest::Client::new();
        let response = client
            .patch(provider.api_url(&format!("/planner/tasks/{}", task_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .header("If-Match", etag)
            .json(&request_body)
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

// =============================================================================
// Create Bucket Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateBucketNode {}

impl CreateBucketNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateBucketNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_create_bucket",
            "Create Bucket",
            "Create a new bucket in a Planner plan",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("plan_id", "Plan ID", "ID of the plan", VariableType::String);
        node.add_input_pin("name", "Name", "Bucket name", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("bucket", "Bucket", "", VariableType::Struct)
            .set_schema::<PlannerBucket>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let plan_id: String = context.evaluate_pin("plan_id").await?;
        let name: String = context.evaluate_pin("name").await?;

        let request_body = json!({
            "planId": plan_id,
            "name": name,
            "orderHint": " !"
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/planner/buckets"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(bucket) = parse_bucket(&body) {
                    context.set_pin_value("bucket", json!(bucket)).await?;
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
// List My Tasks Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListMyTasksNode {}

impl ListMyTasksNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListMyTasksNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_planner_list_my_tasks",
            "List My Tasks",
            "List all Planner tasks assigned to the current user",
            "Data/Microsoft/Planner",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

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
        node.add_output_pin("tasks", "Tasks", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<PlannerTask>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, provider.api_url("/me/planner/tasks"))
            .await
        {
            Ok(values) => {
                let tasks: Vec<PlannerTask> = values.iter().filter_map(parse_task).collect();
                let count = tasks.len() as i64;
                context.set_pin_value("tasks", json!(tasks)).await?;
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
