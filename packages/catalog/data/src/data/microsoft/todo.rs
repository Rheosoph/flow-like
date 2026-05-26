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
// To Do Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TodoTaskList {
    pub id: String,
    pub display_name: String,
    pub is_owner: Option<bool>,
    pub is_shared: Option<bool>,
    pub well_known_list_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TodoTask {
    pub id: String,
    pub title: String,
    pub body_content: Option<String>,
    pub importance: String,
    pub status: String,
    pub created_date_time: Option<String>,
    pub due_date_time: Option<String>,
    pub completed_date_time: Option<String>,
}

fn parse_task_list(value: &Value) -> Option<TodoTaskList> {
    Some(TodoTaskList {
        id: value["id"].as_str()?.to_string(),
        display_name: value["displayName"].as_str()?.to_string(),
        is_owner: value["isOwner"].as_bool(),
        is_shared: value["isShared"].as_bool(),
        well_known_list_name: value["wellknownListName"].as_str().map(String::from),
    })
}

fn parse_task(value: &Value) -> Option<TodoTask> {
    Some(TodoTask {
        id: value["id"].as_str()?.to_string(),
        title: value["title"].as_str()?.to_string(),
        body_content: value["body"]["content"].as_str().map(String::from),
        importance: value["importance"].as_str().unwrap_or("normal").to_string(),
        status: value["status"].as_str().unwrap_or("notStarted").to_string(),
        created_date_time: value["createdDateTime"].as_str().map(String::from),
        due_date_time: value["dueDateTime"]["dateTime"].as_str().map(String::from),
        completed_date_time: value["completedDateTime"]["dateTime"]
            .as_str()
            .map(String::from),
    })
}

// =============================================================================
// List Task Lists Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListTaskListsNode {}

impl ListTaskListsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListTaskListsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_list_lists",
            "List Task Lists",
            "List all Microsoft To Do task lists",
            "Data/Microsoft/To Do",
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
        node.add_output_pin("task_lists", "Task Lists", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<TodoTaskList>();
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
        match graph_get_paginated_values(&client, &provider, provider.api_url("/me/todo/lists"))
            .await
        {
            Ok(values) => {
                let lists: Vec<TodoTaskList> = values.iter().filter_map(parse_task_list).collect();
                let count = lists.len() as i64;
                context.set_pin_value("task_lists", json!(lists)).await?;
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
// Create Task List Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateTaskListNode {}

impl CreateTaskListNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateTaskListNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_create_list",
            "Create Task List",
            "Create a new Microsoft To Do task list",
            "Data/Microsoft/To Do",
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
        node.add_input_pin(
            "display_name",
            "Display Name",
            "Name of the task list",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("task_list", "Task List", "", VariableType::Struct)
            .set_schema::<TodoTaskList>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let display_name: String = context.evaluate_pin("display_name").await?;

        let body = json!({
            "displayName": display_name
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/me/todo/lists"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(list) = parse_task_list(&body) {
                    context.set_pin_value("task_list", json!(list)).await?;
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
// List Tasks Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListTasksNode {}

impl ListTasksNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListTasksNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_list_tasks",
            "List Tasks",
            "List all tasks in a Microsoft To Do task list",
            "Data/Microsoft/To Do",
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
        node.add_input_pin(
            "list_id",
            "List ID",
            "ID of the task list",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("tasks", "Tasks", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<TodoTask>();
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
        let list_id: String = context.evaluate_pin("list_id").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&format!("/me/todo/lists/{}/tasks", list_id)),
        )
        .await
        {
            Ok(values) => {
                let tasks: Vec<TodoTask> = values.iter().filter_map(parse_task).collect();
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
// Create Task Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateTaskNode {}

impl CreateTaskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateTaskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_create_task",
            "Create Task",
            "Create a new task in a Microsoft To Do task list",
            "Data/Microsoft/To Do",
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
        node.add_input_pin(
            "list_id",
            "List ID",
            "ID of the task list",
            VariableType::String,
        );
        node.add_input_pin("title", "Title", "Task title", VariableType::String);
        node.add_input_pin("body", "Body", "Task description", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "importance",
            "Importance",
            "Task importance",
            VariableType::String,
        )
        .set_default_value(Some(json!("normal")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "low".to_string(),
                    "normal".to_string(),
                    "high".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "due_date",
            "Due Date",
            "Due date (YYYY-MM-DD format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("task", "Task", "", VariableType::Struct)
            .set_schema::<TodoTask>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let title: String = context.evaluate_pin("title").await?;
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let importance: String = context
            .evaluate_pin("importance")
            .await
            .unwrap_or_else(|_| "normal".to_string());
        let due_date: String = context.evaluate_pin("due_date").await.unwrap_or_default();

        let mut request_body = json!({
            "title": title,
            "importance": importance
        });

        if !body.is_empty() {
            request_body["body"] = json!({
                "content": body,
                "contentType": "text"
            });
        }

        if !due_date.is_empty() {
            request_body["dueDateTime"] = json!({
                "dateTime": format!("{}T00:00:00", due_date),
                "timeZone": "UTC"
            });
        }

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url(&format!("/me/todo/lists/{}/tasks", list_id)))
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
// Update Task Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdateTaskNode {}

impl UpdateTaskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateTaskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_update_task",
            "Update Task",
            "Update an existing task in Microsoft To Do",
            "Data/Microsoft/To Do",
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
        node.add_input_pin(
            "list_id",
            "List ID",
            "ID of the task list",
            VariableType::String,
        );
        node.add_input_pin("task_id", "Task ID", "ID of the task", VariableType::String);
        node.add_input_pin(
            "title",
            "Title",
            "New task title (leave empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("status", "Status", "Task status", VariableType::String)
            .set_default_value(Some(json!("")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "".to_string(),
                        "notStarted".to_string(),
                        "inProgress".to_string(),
                        "completed".to_string(),
                        "waitingOnOthers".to_string(),
                        "deferred".to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "importance",
            "Importance",
            "Task importance",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "".to_string(),
                    "low".to_string(),
                    "normal".to_string(),
                    "high".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("task", "Task", "", VariableType::Struct)
            .set_schema::<TodoTask>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let task_id: String = context.evaluate_pin("task_id").await?;
        let title: String = context.evaluate_pin("title").await.unwrap_or_default();
        let status: String = context.evaluate_pin("status").await.unwrap_or_default();
        let importance: String = context.evaluate_pin("importance").await.unwrap_or_default();

        let mut request_body = json!({});

        if !title.is_empty() {
            request_body["title"] = json!(title);
        }
        if !status.is_empty() {
            request_body["status"] = json!(status);
        }
        if !importance.is_empty() {
            request_body["importance"] = json!(importance);
        }

        let client = reqwest::Client::new();
        let response = client
            .patch(provider.api_url(&format!("/me/todo/lists/{}/tasks/{}", list_id, task_id)))
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
// Complete Task Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CompleteTaskNode {}

impl CompleteTaskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CompleteTaskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_complete_task",
            "Complete Task",
            "Mark a task as completed in Microsoft To Do",
            "Data/Microsoft/To Do",
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
        node.add_input_pin(
            "list_id",
            "List ID",
            "ID of the task list",
            VariableType::String,
        );
        node.add_input_pin("task_id", "Task ID", "ID of the task", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("task", "Task", "", VariableType::Struct)
            .set_schema::<TodoTask>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Tasks.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let list_id: String = context.evaluate_pin("list_id").await?;
        let task_id: String = context.evaluate_pin("task_id").await?;

        let request_body = json!({
            "status": "completed"
        });

        let client = reqwest::Client::new();
        let response = client
            .patch(provider.api_url(&format!("/me/todo/lists/{}/tasks/{}", list_id, task_id)))
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
// Delete Task Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteTaskNode {}

impl DeleteTaskNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteTaskNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_todo_delete_task",
            "Delete Task",
            "Delete a task from Microsoft To Do",
            "Data/Microsoft/To Do",
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
        node.add_input_pin(
            "list_id",
            "List ID",
            "ID of the task list",
            VariableType::String,
        );
        node.add_input_pin("task_id", "Task ID", "ID of the task", VariableType::String);

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
        let list_id: String = context.evaluate_pin("list_id").await?;
        let task_id: String = context.evaluate_pin("task_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .delete(provider.api_url(&format!("/me/todo/lists/{}/tasks/{}", list_id, task_id)))
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
