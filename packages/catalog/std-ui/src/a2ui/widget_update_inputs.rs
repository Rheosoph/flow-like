use super::elements::element_utils::extract_element_id;
use super::elements::get_element::find_component_in_board;
use super::micro_widget_utils::{
    DYN_INPUT_PREFIX, add_contract_input_pins, collect_prefixed_pin_values,
    connected_widget_contract, expected_contract_input_pin_names, pin_connected,
    remove_stale_prefixed_pins, trace_widget_selector, widget_contract_from_component,
};
use flow_like::a2ui::micro_widget::{
    ResolvedWidget, WidgetContract, WidgetProvider, decode_package_widget_ref,
};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::BTreeSet;

/// Sends a typed props patch to a package (micro) widget instance. The
/// contract comes from the connected widget reference or a selected Page
/// element. Each contract input generates one optional
/// `dyn_in_*` pin. Only pins that are connected or
/// explicitly set are included in the patch.
#[crate::register_node]
#[derive(Default)]
pub struct WidgetUpdateInputs;

impl WidgetUpdateInputs {
    pub fn new() -> Self {
        Self
    }
}

fn reset_input_shape(node: &mut Node) {
    remove_stale_prefixed_pins(node, DYN_INPUT_PREFIX, &BTreeSet::new());
    node.friendly_name = "Update Widget Inputs".to_string();
}

fn configure_input_contract(node: &mut Node, widget_name: &str, contract: &WidgetContract) {
    let widget_name = if widget_name.is_empty() {
        contract.id.as_str()
    } else {
        widget_name
    };
    let expected = expected_contract_input_pin_names(contract);
    remove_stale_prefixed_pins(node, DYN_INPUT_PREFIX, &expected);
    add_contract_input_pins(node, contract, false);
    node.friendly_name = format!("Update {widget_name} Inputs");
}

#[async_trait]
impl NodeLogic for WidgetUpdateInputs {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_widget_update_inputs",
            "Update Widget Inputs",
            "Sends a typed input patch to a package widget instance. Select a Page widget, or connect Element Ref from Instantiate Widget or Element from Get Element, to generate one optional pin per contract input. Only set pins are included in the patch.",
            "UI/Container",
        );
        node.set_flowscript_name("ui", "widgetUpdateInputs");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element Ref",
            "Select a Page widget, or connect its reference from Instantiate Widget or Get Element",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);
        // Keep existing dynamic pin IDs, values, and wires during schema refresh.
        node.set_version(1);

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        if !pin_connected(node, "element_ref") {
            let selected_id = node
                .get_pin_by_name("element_ref")
                .and_then(|pin| pin.default_value.as_ref())
                .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
                .and_then(|value| extract_element_id(&value));
            let Some(element_id) = selected_id else {
                reset_input_shape(node);
                return;
            };

            let Some(element) = find_component_in_board(board, &element_id).await else {
                // A synced board can arrive before its Page files. Preserve configured pins.
                node.error = Some(format!(
                    "Could not load Page widget '{element_id}'. Refresh after the Page is available."
                ));
                return;
            };
            if let Some(metadata) = widget_contract_from_component(&element.component) {
                configure_input_contract(node, &metadata.widget_name, &metadata.contract);
            } else if element.component.get("type").and_then(Value::as_str)
                == Some("microWidgetInstance")
            {
                node.error = Some(format!(
                    "Page widget '{element_id}' has no valid package contract. Refresh the widget from its package."
                ));
            } else {
                reset_input_shape(node);
                node.error = Some(
                    "Update Widget Inputs only supports package widgets. Select a package widget on the Page."
                        .to_string(),
                );
            }
            return;
        }

        if let Some(metadata) = connected_widget_contract(board, node, "element_ref") {
            configure_input_contract(node, &metadata.widget_name, &metadata.contract);
            return;
        }

        // Older boards may carry only the originating Instantiate Widget selector.
        let Some(selector) = trace_widget_selector(board, node, "element_ref") else {
            // References routed through variables can retain a previously configured shape.
            if !node
                .pins
                .values()
                .any(|pin| pin.name.starts_with(DYN_INPUT_PREFIX))
            {
                node.error = Some(
                    "Could not discover this widget's contract. Connect Element from Get Element or Element Ref from Instantiate Widget."
                        .to_string(),
                );
            }
            return;
        };

        if decode_package_widget_ref(&selector).is_none() {
            reset_input_shape(node);
            node.error = Some(
                "Update Widget Inputs only supports package widgets. Use the element Set nodes to update declarative widget internals.".to_string(),
            );
            return;
        }

        let provider = WidgetProvider::from_board(board).await;
        match provider.resolve(&selector) {
            Some(ResolvedWidget::Package(entry)) => match entry.parsed_contract() {
                Ok(contract) => {
                    configure_input_contract(node, &entry.name, &contract);
                }
                Err(e) => {
                    node.error = Some(e.to_string());
                }
            },
            _ => {
                node.error = Some(format!(
                    "The contract for package widget '{selector}' is unavailable. Refresh after the package registry is ready."
                ));
            }
        }
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference: select a Page widget or connect a reference from Get Element or Instantiate Widget"
            )
        })?;

        let props = collect_prefixed_pin_values(context, DYN_INPUT_PREFIX).await;

        if props.is_empty() {
            context.log_message(
                &format!(
                    "No widget inputs set for '{}' — nothing to update",
                    element_id
                ),
                LogLevel::Debug,
            );
        } else {
            context
                .upsert_element(
                    &element_id,
                    json!({
                        "type": "setWidgetProps",
                        "props": props
                    }),
                )
                .await?;
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2ui::elements::get_element::GetElement;
    use crate::a2ui::micro_widget_utils::set_widget_ref_metadata;
    use flow_like::a2ui::{SurfaceComponent, widget::Page};
    use flow_like::flow::pin::ValueType;
    use flow_like::state::{FlowLikeConfig, FlowLikeState};
    use flow_like::utils::http::HTTPClient;
    use flow_like_storage::{
        files::store::FlowLikeStore, object_store::memory::InMemory, object_store::path::Path,
    };
    use std::sync::Arc;

    fn map_contract() -> Value {
        json!({
            "contractVersion": 1,
            "id": "gods-eye-view",
            "inputs": {
                "title": { "type": "string", "default": "God's Eye View" },
                "sceneUpdate": {
                    "type": "json",
                    "schema": { "type": "object", "properties": { "updateId": { "type": "string" } } }
                },
                "layers": { "type": "json", "schema": { "type": "array", "items": { "type": "object" } } }
            }
        })
    }

    fn widget(contract: Value) -> SurfaceComponent {
        SurfaceComponent::new(
            "map",
            json!({
                "type": "microWidgetInstance",
                "packageId": "com.example.geo",
                "packageVersion": "1.0.0",
                "widgetId": "gods-eye-view",
                "instanceId": "placed-map",
                "contract": contract
            }),
        )
    }

    fn page_board() -> Board {
        let mut config = FlowLikeConfig::new();
        config.register_app_meta_store(FlowLikeStore::Other(Arc::new(InMemory::new())));
        let state = Arc::new(FlowLikeState::new(
            config,
            HTTPClient::new_without_refetch(),
        ));
        Board::new(None, Path::from("apps/widget-input-test"), state)
    }

    fn select(node: &mut Node, id: &str) {
        let pin = node.get_pin_mut_by_name("element_ref").unwrap();
        pin.depends_on.clear();
        pin.set_default_value(Some(json!(id)));
    }

    fn assert_map_inputs(node: &Node) {
        assert_eq!(node.error, None);
        let update = node.get_pin_by_name("dyn_in_sceneUpdate").unwrap();
        assert_eq!(update.data_type, VariableType::Struct);
        assert!(update.schema.as_deref().unwrap().contains("updateId"));
        assert_eq!(
            node.get_pin_by_name("dyn_in_layers").unwrap().value_type,
            ValueType::Array
        );
        // A partial update must not reset a title just because the contract supplies a default.
        assert!(
            node.get_pin_by_name("dyn_in_title")
                .unwrap()
                .default_value
                .is_none()
        );
    }

    #[flow_like_types::tokio::test]
    async fn connected_metadata_generates_inputs_without_a_registry() {
        let mut board = Board::new_detached(None, Path::default());
        let mut source = GetElement::new().get_node();
        let output = source.get_pin_mut_by_name("element").unwrap();
        set_widget_ref_metadata(
            output,
            "pkg:com.example.geo/gods-eye-view",
            "God's Eye View",
            &map_contract(),
        );
        let output_id = output.id.clone();
        board.nodes.insert(source.id.clone(), source);
        let mut node = WidgetUpdateInputs::new().get_node();
        node.get_pin_mut_by_name("element_ref")
            .unwrap()
            .depends_on
            .insert(output_id);

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert_map_inputs(&node);
        assert_eq!(node.friendly_name, "Update God's Eye View Inputs");
    }

    #[flow_like_types::tokio::test]
    async fn selected_page_and_get_element_generate_the_same_inputs() {
        let mut board = page_board();
        let other = Page::new("other", "Other", "/other").with_component(widget(json!({
            "contractVersion": 1, "id": "other", "inputs": { "wrong": { "type": "boolean" } }
        })));
        let page = Page::new("page", "Map", "/map").with_component(widget(map_contract()));
        board.save_page(&other, None).await.unwrap();
        board.save_page(&page, None).await.unwrap();
        let mut node = WidgetUpdateInputs::new().get_node();
        select(&mut node, "page/map");

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;
        assert_map_inputs(&node);
        assert!(node.get_pin_by_name("dyn_in_wrong").is_none());
        let pin = node.get_pin_mut_by_name("dyn_in_sceneUpdate").unwrap();
        let pin_id = pin.id.clone();
        pin.depends_on.insert("feed-output".to_string());
        pin.set_default_value(Some(json!({ "updateId": "configured" })));
        let saved_default = pin.default_value.clone();

        let mut get = GetElement::new().get_node();
        select(&mut get, "page/map");
        GetElement::new().on_update(&mut get, &board).await;
        let output_id = get.get_pin_by_name("element").unwrap().id.clone();
        board.nodes.insert(get.id.clone(), get);
        let reference = node.get_pin_mut_by_name("element_ref").unwrap();
        // A connected reference must win over an old dropdown selection.
        reference.set_default_value(Some(json!("other/map")));
        reference.depends_on.insert(output_id);
        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert_map_inputs(&node);
        let pin = node.get_pin_by_name("dyn_in_sceneUpdate").unwrap();
        assert_eq!(pin.id, pin_id);
        assert_eq!(pin.default_value, saved_default);
        assert!(pin.depends_on.contains("feed-output"));
    }

    #[flow_like_types::tokio::test]
    async fn unavailable_page_preserves_configured_inputs_and_reports_it() {
        let board = Board::new_detached(None, Path::default());
        let mut node = WidgetUpdateInputs::new().get_node();
        configure_input_contract(
            &mut node,
            "Map",
            &flow_like_types::json::from_value(map_contract()).unwrap(),
        );
        let pin_id = node
            .get_pin_by_name("dyn_in_sceneUpdate")
            .unwrap()
            .id
            .clone();
        select(&mut node, "missing/map");

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert_eq!(
            node.get_pin_by_name("dyn_in_sceneUpdate").unwrap().id,
            pin_id
        );
        assert!(
            node.error
                .as_deref()
                .unwrap()
                .contains("Could not load Page widget")
        );
    }

    #[flow_like_types::tokio::test]
    async fn unavailable_selected_page_does_not_use_another_pages_contract() {
        let mut board = page_board();
        let other = Page::new("other", "Other", "/other").with_component(widget(json!({
            "contractVersion": 1, "id": "other", "inputs": { "wrong": { "type": "boolean" } }
        })));
        board.save_page(&other, None).await.unwrap();
        board.page_ids.push("missing".to_string());
        let mut node = WidgetUpdateInputs::new().get_node();
        configure_input_contract(
            &mut node,
            "Map",
            &flow_like_types::json::from_value(map_contract()).unwrap(),
        );
        let pin = node.get_pin_mut_by_name("dyn_in_sceneUpdate").unwrap();
        let pin_id = pin.id.clone();
        pin.set_default_value(Some(json!({ "updateId": "configured" })));
        let saved_default = pin.default_value.clone();
        select(&mut node, "missing/map");

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        let pin = node.get_pin_by_name("dyn_in_sceneUpdate").unwrap();
        assert_eq!(pin.id, pin_id);
        assert_eq!(pin.default_value, saved_default);
        assert!(node.get_pin_by_name("dyn_in_wrong").is_none());
        assert!(
            node.error
                .as_deref()
                .unwrap()
                .contains("Could not load Page widget")
        );
    }

    #[flow_like_types::tokio::test]
    async fn untraceable_reference_can_reuse_a_previously_configured_shape() {
        let mut board = Board::new_detached(None, Path::default());
        let mut source = Node::new("test_reference", "Stored Reference", "", "Test");
        let output_id = source
            .add_output_pin("value", "Value", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(source.id.clone(), source);
        let mut node = WidgetUpdateInputs::new().get_node();
        node.get_pin_mut_by_name("element_ref")
            .unwrap()
            .depends_on
            .insert(output_id);

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;
        assert!(
            node.error
                .as_deref()
                .unwrap()
                .contains("Could not discover")
        );

        configure_input_contract(
            &mut node,
            "Map",
            &flow_like_types::json::from_value(map_contract()).unwrap(),
        );
        let pin = node.get_pin_mut_by_name("dyn_in_sceneUpdate").unwrap();
        let pin_id = pin.id.clone();
        pin.depends_on.insert("feed-output".to_string());
        pin.set_default_value(Some(json!({ "updateId": "configured" })));
        let saved_default = pin.default_value.clone();

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert_map_inputs(&node);
        let pin = node.get_pin_by_name("dyn_in_sceneUpdate").unwrap();
        assert_eq!(pin.id, pin_id);
        assert_eq!(pin.default_value, saved_default);
        assert!(pin.depends_on.contains("feed-output"));
    }

    #[flow_like_types::tokio::test]
    async fn unavailable_legacy_registry_preserves_configured_inputs() {
        let mut board = Board::new_detached(None, Path::default());
        let mut source = Node::new(
            "a2ui_instantiate_widget",
            "Instantiate Widget",
            "",
            "UI/Container",
        );
        source
            .add_input_pin("widget_selector", "Widget", "", VariableType::String)
            .set_default_value(Some(json!("pkg:com.example.geo/gods-eye-view")));
        let output_id = source
            .add_output_pin("element_ref", "Element Ref", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(source.id.clone(), source);
        let mut node = WidgetUpdateInputs::new().get_node();
        configure_input_contract(
            &mut node,
            "Map",
            &flow_like_types::json::from_value(map_contract()).unwrap(),
        );
        let pin_id = node
            .get_pin_by_name("dyn_in_sceneUpdate")
            .unwrap()
            .id
            .clone();
        node.get_pin_mut_by_name("element_ref")
            .unwrap()
            .depends_on
            .insert(output_id);

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert_eq!(
            node.get_pin_by_name("dyn_in_sceneUpdate").unwrap().id,
            pin_id
        );
        assert!(node.error.as_deref().unwrap().contains("package registry"));
    }

    #[flow_like_types::tokio::test]
    async fn selecting_non_widget_removes_unwired_inputs_but_preserves_wires() {
        let mut board = page_board();
        let page = Page::new("page", "Page", "/")
            .with_component(SurfaceComponent::new("label", json!({ "type": "text" })));
        board.save_page(&page, None).await.unwrap();
        let mut node = WidgetUpdateInputs::new().get_node();
        configure_input_contract(
            &mut node,
            "Map",
            &flow_like_types::json::from_value(map_contract()).unwrap(),
        );
        let pin = node.get_pin_mut_by_name("dyn_in_sceneUpdate").unwrap();
        let pin_id = pin.id.clone();
        pin.depends_on.insert("feed-output".to_string());
        select(&mut node, "page/label");

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert!(node.get_pin_by_name("dyn_in_title").is_none());
        assert_eq!(
            node.get_pin_by_name("dyn_in_sceneUpdate").unwrap().id,
            pin_id
        );
        assert!(
            node.get_pin_by_name("dyn_in_sceneUpdate")
                .unwrap()
                .depends_on
                .contains("feed-output")
        );
        assert_eq!(node.friendly_name, "Update Widget Inputs");
        assert!(
            node.error
                .as_deref()
                .unwrap()
                .contains("only supports package widgets")
        );
    }

    #[flow_like_types::tokio::test]
    async fn clearing_selection_removes_unwired_inputs() {
        let board = Board::new_detached(None, Path::default());
        let mut node = WidgetUpdateInputs::new().get_node();
        configure_input_contract(
            &mut node,
            "Map",
            &flow_like_types::json::from_value(map_contract()).unwrap(),
        );
        select(&mut node, "");

        WidgetUpdateInputs::new().on_update(&mut node, &board).await;

        assert!(node.get_pin_by_name("dyn_in_sceneUpdate").is_none());
        assert_eq!(node.friendly_name, "Update Widget Inputs");
        assert_eq!(node.error, None);
    }
}
