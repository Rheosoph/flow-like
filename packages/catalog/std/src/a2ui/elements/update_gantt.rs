use super::element_utils::{
    DynamicPinSig, count_matching_pins, extract_element_id, find_element, retain_dynamic_pins,
};
use super::update_schemas::{
    GanttConfig, GanttDependencyUpdate, GanttTask, GanttTaskUpdate, diff_items, ensure_item_id,
    ensure_item_ids,
};
use flow_like::a2ui::components::GanttProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unwrap a component prop's `BoundValue` wrapper into its underlying value.
fn unwrap_bound(value: &Value) -> Value {
    if let Some(obj) = value.as_object() {
        if let Some(json_str) = obj.get("literalJson").and_then(|v| v.as_str())
            && let Ok(parsed) = flow_like_types::json::from_str::<Value>(json_str)
        {
            return parsed;
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
/// - Add Tasks: Append an array of tasks
/// - Update Task: Patch a task by id
/// - Update Tasks: Patch an array of tasks by id
/// - Remove Task: Delete a task by id
/// - Remove Tasks: Delete tasks by an array of ids
/// - Set Progress: Set a task's completion percentage
/// - Add Dependency: Link predecessor -> successor
/// - Remove Dependency: Unlink predecessor -> successor
/// - Set View: Switch day/week/month/quarter/compact
/// - Set Config: Apply a view/behavior config object
/// - Get Tasks: Read current tasks
/// - Diff Tasks: Compare a previous snapshot against current tasks
/// - Get Config: Read current view configuration
#[crate::register_node]
#[derive(Default)]
pub struct UpdateGantt;

const OPERATIONS: &[&str] = &[
    "Set Tasks",
    "Add Task",
    "Add Tasks",
    "Update Task",
    "Update Tasks",
    "Remove Task",
    "Remove Tasks",
    "Set Progress",
    "Add Dependency",
    "Remove Dependency",
    "Set View",
    "Set Config",
    "Get Tasks",
    "Diff Tasks",
    "Get Config",
];

fn operation_options() -> PinOptions {
    PinOptions::new()
        .set_valid_values(OPERATIONS.iter().map(|op| op.to_string()).collect())
        .build()
}

/// Reapply operation-owned schemas without recreating pins, so template pins keep their IDs and
/// connections while still picking up the latest generated schema.
fn refresh_dynamic_schemas(node: &mut Node, operation: &str) {
    match operation {
        "Set Tasks" | "Add Tasks" | "Get Tasks" => {
            for pin in node.pins.values_mut().filter(|pin| pin.name == "tasks") {
                pin.set_schema::<GanttTask>();
            }
        }
        "Add Task" => {
            for pin in node.pins.values_mut().filter(|pin| pin.name == "task") {
                pin.set_schema::<GanttTask>();
            }
        }
        "Update Task" => {
            for pin in node.pins.values_mut().filter(|pin| pin.name == "task") {
                pin.set_schema::<GanttTaskUpdate>();
            }
        }
        "Update Tasks" => {
            for pin in node.pins.values_mut().filter(|pin| pin.name == "tasks") {
                pin.set_schema::<GanttTaskUpdate>();
            }
        }
        "Add Dependency" | "Remove Dependency" => {
            for pin in node
                .pins
                .values_mut()
                .filter(|pin| pin.name == "dependency")
            {
                pin.set_schema::<GanttDependencyUpdate>();
            }
        }
        "Set Config" | "Get Config" => {
            for pin in node.pins.values_mut().filter(|pin| pin.name == "config") {
                pin.set_schema::<GanttConfig>();
            }
        }
        "Diff Tasks" => {
            let schema_pins = ["previous", "created", "updated", "deleted", "current"];
            for pin in node
                .pins
                .values_mut()
                .filter(|pin| schema_pins.contains(&pin.name.as_str()))
            {
                pin.set_schema::<GanttTask>();
            }
        }
        _ => {}
    }
}

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
        node.set_flowscript_name("ui", "updateGantt");
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
        .set_options(operation_options())
        .set_default_value(Some(json!("Set Tasks")));

        node.add_input_pin("tasks", "Tasks", "Array of tasks", VariableType::Struct)
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_schema::<GanttTask>()
            .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);
        node.set_version(2);

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
                let mut tasks: Value = context.evaluate_pin("tasks").await?;
                ensure_item_ids(&mut tasks);
                let update = json!({ "type": "setGanttTasks", "tasks": tasks });
                context.upsert_element(&element_id, update).await?;
            }
            "Add Task" => {
                let pins = context.get_pins_by_name("task").await?;
                for pin in pins {
                    let mut value: Value = context.evaluate_pin_ref(pin).await?;
                    if value.is_null() {
                        continue;
                    }
                    ensure_item_id(&mut value);
                    let task: GanttTask = flow_like_types::json::from_value(value)?;
                    let update = json!({ "type": "addGanttTask", "task": task });
                    context.upsert_element(&element_id, update).await?;
                }
            }
            "Add Tasks" => {
                let mut tasks: Value = context.evaluate_pin("tasks").await?;
                ensure_item_ids(&mut tasks);
                let items = tasks.as_array().cloned().unwrap_or_default();
                for item in items {
                    let update = json!({ "type": "addGanttTask", "task": item });
                    context.upsert_element(&element_id, update).await?;
                }
            }
            "Update Task" => {
                let task: GanttTaskUpdate = context.evaluate_pin("task").await?;
                let update = json!({ "type": "updateGanttTask", "task": task });
                context.upsert_element(&element_id, update).await?;
            }
            "Update Tasks" => {
                let tasks: Value = context.evaluate_pin("tasks").await?;
                let items = tasks.as_array().cloned().unwrap_or_default();
                for item in items {
                    if item.is_null() {
                        continue;
                    }
                    let task: GanttTaskUpdate = flow_like_types::json::from_value(item)?;
                    let update = json!({ "type": "updateGanttTask", "task": task });
                    context.upsert_element(&element_id, update).await?;
                }
            }
            "Remove Task" => {
                let id: String = context.evaluate_pin("task_id").await?;
                let update = json!({ "type": "removeGanttTask", "id": id });
                context.upsert_element(&element_id, update).await?;
            }
            "Remove Tasks" => {
                let ids: Vec<String> = context.evaluate_pin("task_ids").await?;
                for id in ids {
                    let update = json!({ "type": "removeGanttTask", "id": id });
                    context.upsert_element(&element_id, update).await?;
                }
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
            "Diff Tasks" => {
                let previous: Value = context.evaluate_pin("previous").await?;
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let current = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("tasks"))
                    .map(unwrap_bound)
                    .unwrap_or(json!([]));
                let (created, updated, deleted) = diff_items(&previous, &current);
                let changed = !created.is_empty() || !updated.is_empty() || !deleted.is_empty();
                context.set_pin_value("created", json!(created)).await?;
                context.set_pin_value("updated", json!(updated)).await?;
                context.set_pin_value("deleted", json!(deleted)).await?;
                context.set_pin_value("current", current).await?;
                context.set_pin_value("changed", json!(changed)).await?;
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
                    "title",
                    "density",
                    "showViewSwitcher",
                    "showTaskList",
                    "taskListWidth",
                    "shadeWeekends",
                    "columns",
                    "height",
                    "responsive",
                    "compactBreakpoint",
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
        // Refresh the dropdown in place so already-placed nodes pick up newly
        // added operations without a version bump (which would recreate pins).
        if let Some(pin) = node.pins.values_mut().find(|p| p.name == "operation") {
            pin.options = Some(operation_options());
        }

        let operation = node
            .get_pin_by_name("operation")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Set Tasks".to_string());

        let dynamic_pins = [
            "tasks",
            "task",
            "task_id",
            "task_ids",
            "progress",
            "dependency",
            "view",
            "config",
            "count",
            "previous",
            "created",
            "updated",
            "deleted",
            "current",
            "changed",
        ];
        // Expected dynamic pin signatures for the selected operation. Existing
        // matching pins are kept as-is — recreating them would generate new
        // pin ids and sever any connections on every board parse.
        let expected: &[DynamicPinSig] = match operation.as_str() {
            "Set Tasks" | "Add Tasks" => &[("tasks", "Tasks", true)],
            "Add Task" => &[("task", "Task", true)],
            "Update Task" => &[("task", "Task Patch", true)],
            "Update Tasks" => &[("tasks", "Task Patches", true)],
            "Remove Task" => &[("task_id", "Task ID", true)],
            "Remove Tasks" => &[("task_ids", "Task IDs", true)],
            "Set Progress" => &[("task_id", "Task ID", true), ("progress", "Progress", true)],
            "Add Dependency" | "Remove Dependency" => &[("dependency", "Dependency", true)],
            "Set View" => &[("view", "View", true)],
            "Set Config" => &[("config", "Config", true)],
            "Get Tasks" => &[("tasks", "Tasks", false), ("count", "Count", false)],
            "Diff Tasks" => &[
                ("previous", "Previous", true),
                ("created", "Created", false),
                ("updated", "Updated", false),
                ("deleted", "Deleted", false),
                ("current", "Current", false),
                ("changed", "Changed", false),
            ],
            "Get Config" => &[("config", "Config", false)],
            _ => &[],
        };
        retain_dynamic_pins(node, &dynamic_pins, expected);

        match operation.as_str() {
            "Set Tasks" | "Add Tasks" => {
                if count_matching_pins(node, &("tasks", "Tasks", true)) == 0 {
                    node.add_input_pin("tasks", "Tasks", "Array of tasks", VariableType::Struct)
                        .set_value_type(flow_like::flow::pin::ValueType::Array)
                        .set_schema::<GanttTask>()
                        .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
            }
            "Add Task" => {
                // Keep user-added extras from the old multi-pin convention.
                if count_matching_pins(node, &("task", "Task", true)) == 0 {
                    node.add_input_pin("task", "Task", "Task to add", VariableType::Struct)
                        .set_schema::<GanttTask>();
                }
            }
            "Update Task" => {
                if count_matching_pins(node, &("task", "Task Patch", true)) == 0 {
                    node.add_input_pin(
                        "task",
                        "Task Patch",
                        "Fields to change (id required)",
                        VariableType::Struct,
                    )
                    .set_schema::<GanttTaskUpdate>();
                }
            }
            "Update Tasks" => {
                if count_matching_pins(node, &("tasks", "Task Patches", true)) == 0 {
                    node.add_input_pin(
                        "tasks",
                        "Task Patches",
                        "Array of patches (id required per item)",
                        VariableType::Struct,
                    )
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_schema::<GanttTaskUpdate>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
            }
            "Remove Task" => {
                if count_matching_pins(node, &("task_id", "Task ID", true)) == 0 {
                    node.add_input_pin(
                        "task_id",
                        "Task ID",
                        "Id of the task to remove",
                        VariableType::String,
                    );
                }
            }
            "Remove Tasks" => {
                if count_matching_pins(node, &("task_ids", "Task IDs", true)) == 0 {
                    node.add_input_pin(
                        "task_ids",
                        "Task IDs",
                        "Ids of the tasks to remove",
                        VariableType::String,
                    )
                    .set_value_type(flow_like::flow::pin::ValueType::Array);
                }
            }
            "Set Progress" => {
                if count_matching_pins(node, &("task_id", "Task ID", true)) == 0 {
                    node.add_input_pin(
                        "task_id",
                        "Task ID",
                        "Id of the task to update",
                        VariableType::String,
                    );
                }
                if count_matching_pins(node, &("progress", "Progress", true)) == 0 {
                    node.add_input_pin(
                        "progress",
                        "Progress",
                        "Completion percentage (0-100)",
                        VariableType::Float,
                    )
                    .set_default_value(Some(json!(0.0)));
                }
            }
            "Add Dependency" | "Remove Dependency" => {
                if count_matching_pins(node, &("dependency", "Dependency", true)) == 0 {
                    node.add_input_pin(
                        "dependency",
                        "Dependency",
                        "Predecessor -> successor link",
                        VariableType::Struct,
                    )
                    .set_schema::<GanttDependencyUpdate>();
                }
            }
            "Set View" => {
                if count_matching_pins(node, &("view", "View", true)) == 0 {
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
            }
            "Set Config" => {
                if count_matching_pins(node, &("config", "Config", true)) == 0 {
                    node.add_input_pin(
                        "config",
                        "Config",
                        "Gantt view/behavior configuration",
                        VariableType::Struct,
                    )
                    .set_schema::<GanttConfig>();
                }
            }
            "Get Tasks" => {
                if count_matching_pins(node, &("tasks", "Tasks", false)) == 0 {
                    node.add_output_pin(
                        "tasks",
                        "Tasks",
                        "Current gantt tasks",
                        VariableType::Struct,
                    )
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
                if count_matching_pins(node, &("count", "Count", false)) == 0 {
                    node.add_output_pin("count", "Count", "Number of tasks", VariableType::Integer);
                }
            }
            "Diff Tasks" => {
                if count_matching_pins(node, &("previous", "Previous", true)) == 0 {
                    node.add_input_pin(
                        "previous",
                        "Previous",
                        "Snapshot from the last run (empty on first run)",
                        VariableType::Struct,
                    )
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_schema::<GanttTask>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build())
                    .set_default_value(Some(json!([])));
                }
                let outputs = [
                    (
                        "created",
                        "Created",
                        "Tasks present now but missing from the snapshot",
                    ),
                    (
                        "updated",
                        "Updated",
                        "Tasks whose content changed since the snapshot (current version)",
                    ),
                    (
                        "deleted",
                        "Deleted",
                        "Tasks from the snapshot that no longer exist",
                    ),
                    (
                        "current",
                        "Current",
                        "Current tasks — store as the next run's snapshot",
                    ),
                ];
                for (name, friendly, description) in outputs {
                    if count_matching_pins(node, &(name, friendly, false)) == 0 {
                        node.add_output_pin(name, friendly, description, VariableType::Struct)
                            .set_value_type(flow_like::flow::pin::ValueType::Array)
                            .set_schema::<GanttTask>()
                            .set_options(PinOptions::new().set_enforce_schema(false).build());
                    }
                }
                if count_matching_pins(node, &("changed", "Changed", false)) == 0 {
                    node.add_output_pin(
                        "changed",
                        "Changed",
                        "True when any tasks were created, updated or deleted",
                        VariableType::Boolean,
                    );
                }
            }
            "Get Config" if count_matching_pins(node, &("config", "Config", false)) == 0 => {
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

        refresh_dynamic_schemas(node, &operation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_existing_update_tasks_schema_without_replacing_pin() {
        let mut node = UpdateGantt::new().get_node();
        let pin = node
            .get_pin_mut_by_name("tasks")
            .expect("catalog node should contain the tasks pin");
        let original_id = pin.id.clone();
        pin.friendly_name = "Task Patches".to_string();
        pin.schema = Some(r#"{"type":"object","title":"stale"}"#.to_string());
        let mut expected = pin.clone();
        expected.set_schema::<GanttTaskUpdate>();

        refresh_dynamic_schemas(&mut node, "Update Tasks");

        let refreshed = node.get_pin_by_name("tasks").unwrap();
        assert_eq!(refreshed.id, original_id);
        assert_eq!(refreshed.schema, expected.schema);
    }

    #[test]
    fn refreshes_every_legacy_add_task_pin() {
        let mut node = UpdateGantt::new().get_node();
        let first_id = {
            let pin = node.add_input_pin("task", "Task", "", VariableType::Struct);
            pin.schema = Some("stale-one".to_string());
            pin.id.clone()
        };
        let second_id = {
            let pin = node.add_input_pin("task", "Task", "", VariableType::Struct);
            pin.schema = Some("stale-two".to_string());
            pin.id.clone()
        };
        let mut expected = node.get_pin_by_name("task").unwrap().clone();
        expected.set_schema::<GanttTask>();

        refresh_dynamic_schemas(&mut node, "Add Task");

        let refreshed: Vec<_> = node
            .pins
            .values()
            .filter(|pin| pin.name == "task")
            .collect();
        assert_eq!(refreshed.len(), 2);
        assert!(refreshed.iter().all(|pin| pin.schema == expected.schema));
        assert!(refreshed.iter().any(|pin| pin.id == first_id));
        assert!(refreshed.iter().any(|pin| pin.id == second_id));
    }
}
