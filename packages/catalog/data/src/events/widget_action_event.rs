use flow_like::a2ui::micro_widget::{ResolvedWidget, WidgetProvider};
use flow_like::a2ui::widget::{ActionContextPayload, InputValuesPayload};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::BTreeMap;

#[path = "widget_action_schema.rs"]
mod widget_action_schema;
use widget_action_schema::{EventInference, PageTarget, action_context_schema, infer_page};

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
        if board.page_metadata_source() == flow_like::flow::board::PageMetadataSource::Persisted {
            return;
        }
        node.error = None;
        let selected = node
            .get_pin_by_name("action_id")
            .and_then(|pin| pin.default_value.as_ref())
            .and_then(|value| flow_like_types::json::from_slice::<String>(value).ok())
            .unwrap_or_default();
        let provider = WidgetProvider::from_board(board).await;
        let mut inference = EventInference::default();

        // Several instances may share an event node. Keep every possible source.
        for instance in board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        {
            if !instance
                .fn_refs
                .as_ref()
                .is_some_and(|refs| refs.fn_refs.contains(&node.id))
            {
                continue;
            }
            // Other function callers invoke this entry without a widget payload.
            if instance.name != "a2ui_instantiate_widget" {
                inference.untyped_source();
                continue;
            }
            let selector = instance
                .get_pin_by_name("widget_selector")
                .and_then(|pin| pin.default_value.as_ref())
                .and_then(|value| flow_like_types::json::from_slice::<String>(value).ok());
            match selector
                .as_deref()
                .and_then(|selector| provider.resolve(selector))
            {
                Some(ResolvedWidget::Package(entry)) => match entry.parsed_contract() {
                    Ok(contract) => {
                        let events = contract
                            .events
                            .into_iter()
                            .map(|(name, event)| (name, event.payload_schema))
                            .collect();
                        inference.instantiated_events(&events, &selected);
                    }
                    Err(_) => inference.unknown_source(),
                },
                Some(ResolvedWidget::Declarative(widget)) => {
                    let events = widget
                        .actions
                        .iter()
                        .map(|action| (action.id.clone(), None))
                        .collect();
                    inference.instantiated_events(&events, &selected);
                }
                None => inference.unknown_source(),
            }
        }

        if !board.page_ids.is_empty() {
            match board.load_pages_for_metadata(None).await {
                Ok(loaded) => {
                    let mut definitions: BTreeMap<String, Value> = BTreeMap::new();
                    let mut contracts = BTreeMap::new();
                    // Published pages must use their embedded definitions; the
                    // currently installed app packages can describe a newer revision.
                    if board.page_metadata_source()
                        == flow_like::flow::board::PageMetadataSource::Draft
                    {
                        definitions = provider
                            .declarative_widgets()
                            .iter()
                            .filter_map(|widget| {
                                flow_like_types::json::to_value(widget)
                                    .ok()
                                    .map(|value| (widget.id.clone(), value))
                            })
                            .collect();
                        contracts = provider
                            .package_widgets()
                            .iter()
                            .map(|entry| (entry.selector(), entry.contract.clone()))
                            .collect();
                    }
                    let app_id = board.board_dir.filename().unwrap_or_default();
                    let target = PageTarget {
                        app_id,
                        board_id: &board.id,
                        node_id: &node.id,
                    };
                    for page in loaded.pages {
                        if let Ok(page) = flow_like_types::json::to_value(page) {
                            infer_page(&page, &target, &definitions, &contracts, &mut inference);
                        }
                    }
                    if !loaded.unreadable.is_empty() {
                        inference.unknown_source();
                    }
                }
                Err(_) => inference.unknown_source(),
            }
        }
        apply_inference(node, &selected, inference);
    }
}

fn apply_inference(node: &mut Node, selected: &str, inference: EventInference) {
    node.error = None;
    let action_ids: Vec<String> = inference.action_ids.iter().cloned().collect();
    if inference.is_complete()
        && !selected.is_empty()
        && !action_ids.is_empty()
        && !inference.action_ids.contains(selected)
    {
        node.error = Some(format!(
            "Event '{selected}' is not defined by the widgets bound to this node"
        ));
    }
    if let Some(pin) = node.get_pin_mut_by_name("action_id") {
        let mut options = pin.options.clone().unwrap_or_else(PinOptions::new);
        options.valid_values = (!action_ids.is_empty()).then_some(action_ids);
        pin.set_options(options);
    }
    if let Some(pin) = node.get_pin_mut_by_name("action_context") {
        match inference.payload_schema() {
            Some(schema) => {
                pin.schema = flow_like_types::json::to_string(&action_context_schema(schema)).ok()
            }
            None => {
                pin.set_schema::<ActionContextPayload>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_last_binding_clears_previous_schema_options_and_error() {
        let mut node = WidgetActionEvent::new().get_node();
        let generic = node
            .get_pin_by_name("action_context")
            .unwrap()
            .schema
            .clone();
        let schema = json!({"type":"object","properties":{"entityId":{"type":"string"}},"required":["entityId"]});
        let mut bound = EventInference::default();
        bound.add("entityClicked", Some(&schema));
        apply_inference(&mut node, "entityClicked", bound);
        assert_eq!(
            node.get_pin_by_name("action_id")
                .unwrap()
                .options
                .as_ref()
                .unwrap()
                .valid_values
                .as_ref()
                .unwrap(),
            &["entityClicked"]
        );
        assert_eq!(
            flow_like_types::json::from_str::<Value>(
                node.get_pin_by_name("action_context")
                    .unwrap()
                    .schema
                    .as_deref()
                    .unwrap()
            )
            .unwrap(),
            action_context_schema(&schema)
        );
        node.error = Some("stale binding error".into());
        apply_inference(&mut node, "entityClicked", EventInference::default());
        assert_eq!(
            node.get_pin_by_name("action_context").unwrap().schema,
            generic
        );
        assert!(
            node.get_pin_by_name("action_id")
                .unwrap()
                .options
                .as_ref()
                .unwrap()
                .valid_values
                .is_none()
        );
        assert!(node.error.is_none());
    }

    #[test]
    fn incomplete_source_discovery_cannot_reject_a_potentially_valid_event() {
        let mut node = WidgetActionEvent::new().get_node();
        let mut known = EventInference::default();
        known.add("clicked", None);
        apply_inference(&mut node, "unresolved-event", known);
        assert!(node.error.is_some());
        let mut partial = EventInference::default();
        partial.add("clicked", None);
        partial.unknown_source();
        apply_inference(&mut node, "unresolved-event", partial);
        assert!(node.error.is_none());
    }

    #[tokio::test]
    async fn saved_page_bindings_refresh_the_real_event_node_and_respect_published_versions() {
        use flow_like::{
            a2ui::{SurfaceComponent, widget::Page},
            flow::pin::resolve_schema,
            state::{FlowLikeConfig, FlowLikeState},
            utils::http::HTTPClient,
        };
        use flow_like_storage::{
            files::store::FlowLikeStore,
            object_store::{memory::InMemory, path::Path},
        };
        use flow_like_types::{FromProto, ToProto};
        use std::sync::Arc;

        let mut config = FlowLikeConfig::new();
        config.register_app_meta_store(FlowLikeStore::Other(Arc::new(InMemory::new())));
        let state = Arc::new(FlowLikeState::new(
            config,
            HTTPClient::new_without_refetch(),
        ));
        let logic = Arc::new(WidgetActionEvent::new());
        state.node_registry().write().await.push_node(logic.clone());
        let mut board = Board::new(None, Path::from("apps/page-app"), state.clone());
        let node = logic.get_node();
        let node_id = node.id.clone();
        let generic = flow_like_types::json::from_str::<Value>(
            node.get_pin_by_name("action_context")
                .unwrap()
                .schema
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        board.nodes.insert(node_id.clone(), node);
        let clicked = json!({"type":"object","properties":{"entityId":{"type":"string"}},"required":["entityId"]});
        let moved = json!({"type":"object","properties":{"longitude":{"type":"number"}},"required":["longitude"]});
        let clicked_context = action_context_schema(&clicked);
        let moved_context = action_context_schema(&moved);
        let route = json!({"name":"workflow_event","context":{"nodeId":{"literalString":node_id}}});
        let mut page = Page::new("page", "Map", "/map").with_component(SurfaceComponent::new("map", json!({
            "type":"microWidgetInstance","packageId":"geo","widgetId":"map","instanceId":"placed-map",
            "contract":{"id":"map","events":{"clicked":{"payloadSchema":clicked},"moved":{"payloadSchema":moved}}},
            "eventHandlers":{"clicked":[route]}
        })));
        let schema = |board: &Board| {
            let raw = board.nodes[&node_id]
                .get_pin_by_name("action_context")
                .unwrap()
                .schema
                .as_deref()
                .unwrap();
            flow_like_types::json::from_str::<Value>(resolve_schema(raw, &board.refs).unwrap())
                .unwrap()
        };
        let options = |board: &Board| {
            board.nodes[&node_id]
                .get_pin_by_name("action_id")
                .unwrap()
                .options
                .as_ref()
                .and_then(|options| options.valid_values.clone())
                .unwrap_or_default()
        };
        board.save_page(&page, None).await.unwrap();
        assert_eq!(schema(&board), clicked_context);
        assert_eq!(options(&board), vec!["clicked"]);
        let version = board.version;
        board.snapshot_at_version(version, None).await.unwrap();

        page.components[0].component["eventHandlers"] = json!({"moved":[route]});
        board.save_page(&page, None).await.unwrap();
        assert_eq!(schema(&board), moved_context);
        assert_eq!(options(&board), vec!["moved"]);
        let published = Board::load(
            board.board_dir.clone(),
            &board.id,
            state.clone(),
            Some(version),
        )
        .await
        .unwrap();
        assert_eq!(schema(&published), clicked_context);
        assert_eq!(options(&published), vec!["clicked"]);

        let mut referrer = Node::new("agent_register_tools", "Register Tools", "", "test");
        referrer.set_can_reference_fns(true);
        referrer
            .fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(node_id.clone());
        board.nodes.insert(referrer.id.clone(), referrer.clone());
        let mut referenced = board.nodes[&node_id].clone();
        logic.on_update(&mut referenced, &board).await;
        board.nodes.insert(node_id.clone(), referenced);
        assert_eq!(schema(&board), generic);
        assert_eq!(options(&board), vec!["moved"]);
        board.nodes.remove(&referrer.id);

        page.components[0].component["eventHandlers"] = json!({});
        board.save_page(&page, None).await.unwrap();
        assert_eq!(schema(&board), generic);
        assert!(options(&board).is_empty());
        board.delete_page(&page.id, None).await.unwrap();
        assert_eq!(schema(&board), generic);
        let persisted =
            Board::from_loaded_proto(published.to_proto(), board.board_dir.clone(), state)
                .await
                .unwrap();
        assert_eq!(schema(&persisted), clicked_context);
        assert_eq!(options(&persisted), vec!["clicked"]);
        let restored = Board::from_proto(board.to_proto());
        assert_eq!(schema(&restored), generic);
        assert!(options(&restored).is_empty());
    }
}
