use super::element_utils::{extract_element_id, find_element};
use super::update_schemas::{CalendarConfig, CalendarEvent, CalendarEventUpdate};
use flow_like::a2ui::components::CalendarProps;
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

/// Unified Calendar update node.
///
/// Manage a calendar element's events and view configuration with a single
/// node. Input pins change dynamically based on the selected operation.
///
/// **Operations:**
/// - Set Events: Replace all events
/// - Add Event: Append a single event
/// - Update Event: Patch an event by id
/// - Remove Event: Delete an event by id
/// - Set View: Switch month/week/day/agenda
/// - Set Date: Focus a specific date
/// - Set Config: Apply a view/behavior config object
/// - Get Events: Read current events
/// - Get Config: Read current view configuration
#[crate::register_node]
#[derive(Default)]
pub struct UpdateCalendar;

impl UpdateCalendar {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateCalendar {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_calendar",
            "Update Calendar",
            "Add, remove, or update calendar events and view configuration",
            "UI/Elements/Calendar",
        );
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Calendar",
            "Reference to the calendar element",
            VariableType::Struct,
        )
        .set_schema::<CalendarProps>()
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
                    "Set Events".to_string(),
                    "Add Event".to_string(),
                    "Update Event".to_string(),
                    "Remove Event".to_string(),
                    "Set View".to_string(),
                    "Set Date".to_string(),
                    "Set Config".to_string(),
                    "Get Events".to_string(),
                    "Get Config".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Set Events")));

        node.add_input_pin("events", "Events", "Array of events", VariableType::Struct)
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_schema::<CalendarEvent>()
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
            "Set Events" => {
                let events: Value = context.evaluate_pin("events").await?;
                let update = json!({ "type": "setCalendarEvents", "events": events });
                context.upsert_element(&element_id, update).await?;
            }
            "Add Event" => {
                let event: CalendarEvent = context.evaluate_pin("event").await?;
                let update = json!({ "type": "addCalendarEvent", "event": event });
                context.upsert_element(&element_id, update).await?;
            }
            "Update Event" => {
                let event: CalendarEventUpdate = context.evaluate_pin("event").await?;
                let update = json!({ "type": "updateCalendarEvent", "event": event });
                context.upsert_element(&element_id, update).await?;
            }
            "Remove Event" => {
                let id: String = context.evaluate_pin("event_id").await?;
                let update = json!({ "type": "removeCalendarEvent", "id": id });
                context.upsert_element(&element_id, update).await?;
            }
            "Set View" => {
                let view: String = context.evaluate_pin("view").await?;
                let update = json!({ "type": "setCalendarView", "view": view });
                context.upsert_element(&element_id, update).await?;
            }
            "Set Date" => {
                let date: String = context.evaluate_pin("date").await?;
                let update = json!({ "type": "setCalendarDate", "date": date });
                context.upsert_element(&element_id, update).await?;
            }
            "Set Config" => {
                let config: CalendarConfig = context.evaluate_pin("config").await?;
                let update = json!({ "type": "setCalendarConfig", "config": config });
                context.upsert_element(&element_id, update).await?;
            }
            "Get Events" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let events = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("events"))
                    .map(unwrap_bound)
                    .unwrap_or(json!([]));
                let count = events.as_array().map(|a| a.len()).unwrap_or(0);
                context.set_pin_value("events", events).await?;
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
                    "date",
                    "editable",
                    "selectable",
                    "firstDayOfWeek",
                    "minTime",
                    "maxTime",
                    "slotDuration",
                    "showWeekends",
                    "showNowIndicator",
                    "showAllDay",
                    "locale",
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
            .unwrap_or_else(|| "Set Events".to_string());

        let dynamic_pins = [
            "events", "event", "event_id", "view", "date", "config", "count",
        ];
        for pin_name in dynamic_pins {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Set Events" => {
                node.add_input_pin("events", "Events", "Array of events", VariableType::Struct)
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_schema::<CalendarEvent>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
            }
            "Add Event" => {
                node.add_input_pin("event", "Event", "Event to add", VariableType::Struct)
                    .set_schema::<CalendarEvent>();
            }
            "Update Event" => {
                node.add_input_pin(
                    "event",
                    "Event Patch",
                    "Fields to change (id required)",
                    VariableType::Struct,
                )
                .set_schema::<CalendarEventUpdate>();
            }
            "Remove Event" => {
                node.add_input_pin(
                    "event_id",
                    "Event ID",
                    "Id of the event to remove",
                    VariableType::String,
                );
            }
            "Set View" => {
                node.add_input_pin("view", "View", "Calendar view", VariableType::String)
                    .set_options(
                        PinOptions::new()
                            .set_valid_values(vec![
                                "month".to_string(),
                                "week".to_string(),
                                "day".to_string(),
                                "agenda".to_string(),
                            ])
                            .build(),
                    )
                    .set_default_value(Some(json!("month")));
            }
            "Set Date" => {
                node.add_input_pin(
                    "date",
                    "Date",
                    "Focused date (ISO 8601)",
                    VariableType::String,
                );
            }
            "Set Config" => {
                node.add_input_pin(
                    "config",
                    "Config",
                    "Calendar view/behavior configuration",
                    VariableType::Struct,
                )
                .set_schema::<CalendarConfig>();
            }
            "Get Events" => {
                node.add_output_pin(
                    "events",
                    "Events",
                    "Current calendar events",
                    VariableType::Struct,
                )
                .set_value_type(flow_like::flow::pin::ValueType::Array)
                .set_options(PinOptions::new().set_enforce_schema(false).build());
                node.add_output_pin("count", "Count", "Number of events", VariableType::Integer);
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
