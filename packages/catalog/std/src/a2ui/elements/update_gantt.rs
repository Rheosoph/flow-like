use super::element_utils::{extract_element_id, find_element};
use super::update_schemas::{GanttConfig, GanttDependencyUpdate, GanttTask, GanttTaskUpdate};
use flow_like::a2ui::components::GanttProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unwrap a component prop's `BoundValue` wrapper into its underlying value.
fn unwrap_bound(value: &Value) -> Value {
    if let Some(obj) = value.as_object() {
        if let Some(json_str) = obj.get("literalJson").and_then(|v| v.as_str()) {
            if let Ok(parsed) = flow_like_types::json::from_str::<Value>(json_str) {
                return parsed;
            }
        }
        for key in [
            "literalString",
            "literalNumber",
            "literalBool",
            "literalOptions",
        ] {
            if let Some(inner) = obj.get(key) {
                return inner.clone();
            }
        }
    }
    value.clone()
}

/// Unified Gantt update node.
///
/// Manage a gantt element's tasks, dependencies and view configuration with a
/// single node. Input pins change dynamically based on the selected operation.
///
/// **Operations:**
/// - Set Tasks: Replace all tasks
/// - Add Task: Append a single task
/// - Update Task: Patch a task by id
/// - Remove Task: Delete a task by id
/// - Set Progress: Set a task's completion percentage
/// - Add Dependency: Link predecessor -> successor
/// - Remove Dependency: Unlink predecessor -> successor
/// - Set View: Switch day/week/month/quarter/compact
/// - Set Config: Apply a view/behavior config object
/// - Get Tasks: Read current tasks
/// - Get Config: Read current view configuration
#[crate::register_node]
#[derive(Default)]
pub struct UpdateGantt;

impl UpdateGantt {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateGantt {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_gantt",
            "Update Gantt",
            "Add, remove, or update gantt tasks, dependencies and configuration",
            "UI/Elements/Gantt",
        );
        node.add_icon("/flow/icons/gantt.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Gantt",
            "Reference to the gantt element",
            VariableType::Struct,
        )
        .set_schema::<GanttProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "operation",
            "Operation",
            "What operation to perform",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Set Tasks".to_string(),
                    "Add Task".to_string(),
                    "Update Task".to_string(),
                    "Remove Task".to_string(),
                    "Set Progress".to_string(),
                    "Add Dependency".to_string(),
                    "Remove Dependency".to_string(),
                    "Set View".to_string(),
                    "Set Config".to_string(),
                    "Get Tasks".to_string(),
                    "Get Config".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Set Tasks")));

        node.add_input_pin("tasks", "Tasks", "Array of tasks", VariableType::Struct)
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_schema::<GanttTask>()
            .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let operation: String = context.evaluate_pin("operation").await?;

        match operation.as_str() {
            "Set Tasks" => {
                let tasks: Value = context.evaluate_pin("tasks").await?;
                let update = json!({ "type": "setGanttTasks", "tasks": tasks });
                context.upsert_element(&element_id, update).await?;
            }
            "Add Task" => {
                let task: GanttTask = context.evaluate_pin("task").await?;
                let update = json!({ "type": "addGanttTask", "task": task });
                context.upsert_element(&element_id, update).await?;
            }
            "Update Task" => {
                let task: GanttTaskUpdate = context.evaluate_pin("task").await?;
                let update = json!({ "type": "updateGanttTask", "task": task });
                context.upsert_element(&element_id, update).await?;
            }
            "Remove Task" => {
                let id: String = context.evaluate_pin("task_id").await?;
                let update = json!({ "type": "removeGanttTask", "id": id });
                context.upsert_element(&element_id, update).await?;
            }
            "Set Progress" => {
                let id: String = context.evaluate_pin("task_id").await?;
                let progress: f64 = context.evaluate_pin("progress").await?;
                let update = json!({ "type": "setGanttProgress", "id": id, "progress": progress });
                context.upsert_element(&element_id, update).await?;
            }
            "Add Dependency" => {
                let dep: GanttDependencyUpdate = context.evaluate_pin("dependency").await?;
                let update = json!({
                    "type": "addGanttDependency",
                    "fromId": dep.from_id,
                    "toId": dep.to_id,
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Remove Dependency" => {
                let dep: GanttDependencyUpdate = context.evaluate_pin("dependency").await?;
                let update = json!({
                    "type": "removeGanttDependency",
                    "fromId": dep.from_id,
                    "toId": dep.to_id,
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Set View" => {
                let view: String = context.evaluate_pin("view").await?;
                let update = json!({ "type": "setGanttView", "view": view });
                context.upsert_element(&element_id, update).await?;
            }
            "Set Config" => {
                let config: GanttConfig = context.evaluate_pin("config").await?;
                let update = json!({ "type": "setGanttConfig", "config": config });
                context.upsert_element(&element_id, update).await?;
            }
            "Get Tasks" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let tasks = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("tasks"))
                    .map(unwrap_bound)
                    .unwrap_or(json!([]));
                let count = tasks.as_array().map(|a| a.len()).unwrap_or(0);
                context.set_pin_value("tasks", tasks).await?;
                context.set_pin_value("count", json!(count)).await?;
            }
            "Get Config" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let component = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .cloned()
                    .unwrap_or(json!({}));
                let keys = [
                    "view",
                    "editable",
                    "draggable",
                    "resizable",
                    "showDependencies",
                    "showProgress",
                    "showToday",
                    "rowHeight",
                ];
                let mut config = flow_like_types::json::Map::new();
                for key in keys {
                    if let Some(value) = component.get(key) {
                        config.insert(key.to_string(), unwrap_bound(value));
                    }
                }
                context
                    .set_pin_value("config", Value::Object(config))
                    .await?;
            }
            _ => return Err(flow_like_types::anyhow!("Unknown operation: {}", operation)),
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let operation = node
            .get_pin_by_name("operation")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Set Tasks".to_string());

        let dynamic_pins = [
            "tasks",
            "task",
            "task_id",
            "progress",
            "dependency",
            "view",
            "config",
            "count",
        ];
        for pin_name in dynamic_pins {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Set Tasks" => {
                node.add_input_pin("tasks", "Tasks", "Array of tasks", VariableType::Struct)
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_schema::<GanttTask>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
            }
            "Add Task" => {
                node.add_input_pin("task", "Task", "Task to add", VariableType::Struct)
                    .set_schema::<GanttTask>();
            }
            "Update Task" => {
                node.add_input_pin(
                    "task",
                    "Task Patch",
                    "Fields to change (id required)",
                    VariableType::Struct,
                )
                .set_schema::<GanttTaskUpdate>();
            }
            "Remove Task" => {
                node.add_input_pin(
                    "task_id",
                    "Task ID",
                    "Id of the task to remove",
                    VariableType::String,
                );
            }
            "Set Progress" => {
                node.add_input_pin(
                    "task_id",
                    "Task ID",
                    "Id of the task to update",
                    VariableType::String,
                );
                node.add_input_pin(
                    "progress",
                    "Progress",
                    "Completion percentage (0-100)",
                    VariableType::Float,
                )
                .set_default_value(Some(json!(0.0)));
            }
            "Add Dependency" | "Remove Dependency" => {
                node.add_input_pin(
                    "dependency",
                    "Dependency",
                    "Predecessor -> successor link",
                    VariableType::Struct,
                )
                .set_schema::<GanttDependencyUpdate>();
            }
            "Set View" => {
                node.add_input_pin("view", "View", "Timeline zoom", VariableType::String)
                    .set_options(
                        PinOptions::new()
                            .set_valid_values(vec![
                                "day".to_string(),
                                "week".to_string(),
                                "month".to_string(),
                                "quarter".to_string(),
                                "compact".to_string(),
                            ])
                            .build(),
                    )
                    .set_default_value(Some(json!("week")));
            }
            "Set Config" => {
                node.add_input_pin(
                    "config",
                    "Config",
                    "Gantt view/behavior configuration",
                    VariableType::Struct,
                )
                .set_schema::<GanttConfig>();
            }
            "Get Tasks" => {
                node.add_output_pin(
                    "tasks",
                    "Tasks",
                    "Current gantt tasks",
                    VariableType::Struct,
                )
                .set_value_type(flow_like::flow::pin::ValueType::Array)
                .set_options(PinOptions::new().set_enforce_schema(false).build());
                node.add_output_pin("count", "Count", "Number of tasks", VariableType::Integer);
            }
            "Get Config" => {
                node.add_output_pin(
                    "config",
                    "Config",
                    "Current view configuration",
                    VariableType::Struct,
                )
                .set_options(PinOptions::new().set_enforce_schema(false).build());
            }
            _ => {}
        }
    }
}
