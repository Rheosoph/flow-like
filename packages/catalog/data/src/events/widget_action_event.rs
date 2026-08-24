use flow_like::a2ui::micro_widget::{ResolvedWidget, WidgetProvider, decode_package_widget_ref};
use flow_like::a2ui::widget::{ActionContextPayload, InputValuesPayload, Widget};
use flow_like::app::App;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Widget Action Event - Entry point for widget action triggers.
///
/// This node acts as an entry point when a widget action is triggered in the UI.
/// The action context (data provided by the widget) is passed through the payload
/// and can be accessed via output pins.
///
/// Use this instead of Simple Event when you need context from widget actions.
#[crate::register_node]
#[derive(Default)]
pub struct WidgetActionEvent;

impl WidgetActionEvent {
    pub fn new() -> Self {
        Self
    }
}

fn find_widget_by_selector<'a>(widgets: &'a [Widget], selector: &str) -> Option<&'a Widget> {
    widgets
        .iter()
        .find(|w| w.id == selector)
        .or_else(|| widgets.iter().find(|w| w.name == selector))
}

#[async_trait]
impl NodeLogic for WidgetActionEvent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_widget_action",
            "Widget Action Event",
            "Entry point triggered when a widget action is invoked. Provides action context data.",
            "Events",
        );
        node.set_flowscript_name("events", "widgetAction");
        node.add_icon("/flow/icons/event.svg");
        node.set_start(true);
        node.set_can_be_referenced_by_fns(true);

        node.add_input_pin(
            "action_id",
            "Action ID",
            "The action identifier that triggers this event (e.g., 'clicked_delete', 'clicked_open')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Triggered when the widget action is invoked",
            VariableType::Execution,
        );

        node.add_output_pin(
            "widget_instance_id",
            "Widget Instance ID",
            "The unique ID of the widget instance that triggered the action",
            VariableType::String,
        );

        node.add_output_pin(
            "event_name",
            "Event Name",
            "The action ID / event name that was triggered",
            VariableType::String,
        );

        node.add_output_pin(
            "action_context",
            "Action Context",
            "The context data passed from the widget action (JSON object with field values)",
            VariableType::Struct,
        )
        .set_schema::<ActionContextPayload>();

        node.add_output_pin(
            "input_values",
            "Input Values",
            "Map of component ID to current value for components marked as event-relevant",
            VariableType::Struct,
        )
        .set_schema::<InputValuesPayload>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let payload = context.get_payload().await?;

        // Extract widget action context from payload
        let widget_instance_id = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_widget_instance_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let action_id = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_action_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let action_context = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_action_context"))
            .cloned()
            .unwrap_or(json!({}));

        let input_values = payload
            .payload
            .as_ref()
            .and_then(|p| p.get("_input_values"))
            .cloned()
            .unwrap_or(json!({}));

        // Set output pins
        context
            .get_pin_by_name("widget_instance_id")
            .await?
            .set_value(json!(widget_instance_id))
            .await;

        context
            .get_pin_by_name("event_name")
            .await?
            .set_value(json!(action_id))
            .await;

        context
            .get_pin_by_name("action_context")
            .await?
            .set_value(action_context)
            .await;

        context
            .get_pin_by_name("input_values")
            .await?
            .set_value(input_values)
            .await;

        // Activate execution flow
        let exec_out_pin = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out_pin).await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        // Find the InstantiateWidget that references this event via fn_refs
        let referencing_node = board.nodes.values().find(|n| {
            n.name == "a2ui_instantiate_widget"
                && n.fn_refs
                    .as_ref()
                    .is_some_and(|refs| refs.fn_refs.contains(&node.id))
        });

        if let Some(inst_node) = referencing_node {
            // Read the selected widget ID/name from the InstantiateWidget
            let selected_widget = inst_node
                .get_pin_by_name("widget_selector")
                .and_then(|p| p.default_value.as_ref())
                .and_then(|v| flow_like_types::json::from_slice::<String>(v).ok());

            if let Some(widget_selector) = selected_widget {
                // Package widget: contract events take the place of widget actions
                if decode_package_widget_ref(&widget_selector).is_some() {
                    let provider = WidgetProvider::from_board(board).await;
                    if let Some(ResolvedWidget::Package(entry)) = provider.resolve(&widget_selector)
                        && let Ok(contract) = entry.parsed_contract()
                    {
                        let event_names: Vec<String> = contract.events.keys().cloned().collect();

                        if let Some(pin) = node.get_pin_mut_by_name("action_id") {
                            pin.set_options(
                                PinOptions::new()
                                    .set_valid_values(event_names.clone())
                                    .build(),
                            );
                        }

                        let action_id = node
                            .get_pin_by_name("action_id")
                            .and_then(|p| p.default_value.as_ref())
                            .and_then(|v| flow_like_types::json::from_slice::<String>(v).ok())
                            .unwrap_or_default();

                        if !action_id.is_empty()
                            && !event_names.is_empty()
                            && !event_names.contains(&action_id)
                        {
                            node.error = Some(format!(
                                "Event '{}' is not defined on widget '{}'",
                                action_id, entry.name
                            ));
                        }

                        // Type the payload output pin with the contract event's payloadSchema
                        let payload_schema = contract
                            .events
                            .get(&action_id)
                            .and_then(|event| event.payload_schema.clone())
                            .filter(|schema| !schema.is_null());
                        if let Some(pin) = node.get_pin_mut_by_name("action_context") {
                            match payload_schema {
                                Some(schema) => {
                                    pin.schema = flow_like_types::json::to_string(&schema).ok();
                                }
                                None => {
                                    pin.set_schema::<ActionContextPayload>();
                                }
                            }
                        }
                    }
                    return;
                }

                // Load the app's widgets to get available actions
                if let Some(app_state) = &board.app_state {
                    let app_id = board.board_dir.filename().unwrap_or_default().to_string();
                    if !app_id.is_empty()
                        && let Ok(app) = App::load(app_id, app_state.clone()).await
                    {
                        let widgets = app.get_widgets().await.unwrap_or_default();
                        if let Some(widget) = find_widget_by_selector(&widgets, &widget_selector) {
                            let action_ids: Vec<String> =
                                widget.actions.iter().map(|a| a.id.clone()).collect();

                            if let Some(pin) = node.get_pin_mut_by_name("action_id") {
                                pin.set_options(
                                    PinOptions::new()
                                        .set_valid_values(action_ids.clone())
                                        .build(),
                                );
                            }

                            // Validate current action_id against available actions
                            let action_id = node
                                .get_pin_by_name("action_id")
                                .and_then(|p| p.default_value.as_ref())
                                .and_then(|v| flow_like_types::json::from_slice::<String>(v).ok())
                                .unwrap_or_default();

                            if !action_id.is_empty()
                                && !action_ids.is_empty()
                                && !action_ids.contains(&action_id)
                            {
                                node.error = Some(format!(
                                    "Action '{}' is not defined on widget '{}'",
                                    action_id, widget.name
                                ));
                            }

                            // Declarative widgets use the generic action context payload
                            if let Some(pin) = node.get_pin_mut_by_name("action_context") {
                                pin.set_schema::<ActionContextPayload>();
                            }

                            return;
                        }
                    }
                }
            }
        }

        // Fallback: no referencing InstantiateWidget found.
        // Empty action_id is valid — it acts as a catch-all handler for all widget actions.
    }
}
