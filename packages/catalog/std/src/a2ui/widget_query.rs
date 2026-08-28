use super::elements::element_utils::extract_element_id;
use super::micro_widget_utils::{
    DYN_ARG_PREFIX, connected_widget_contract, json_schema_pin_shape, pin_connected,
    remove_stale_prefixed_pins, trace_widget_selector,
};
use flow_like::a2ui::micro_widget::{
    ContractQuery, ResolvedWidget, WidgetContract, WidgetProvider, decode_package_widget_ref,
};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{Pin, PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::collections::BTreeSet;

/// How long the node waits for a live surface to answer before falling back
/// to the value:changed mirror.
const LIVE_QUERY_TIMEOUT_MS: u64 = 10_000;

/// Runs a contract query against a package (micro) widget instance. A mounted
/// widget answers through the live request/response channel; headless and
/// closed-surface runs fall back to values mirrored through `value:changed`.
#[crate::register_node]
#[derive(Default)]
pub struct WidgetQuery;

impl WidgetQuery {
    pub fn new() -> Self {
        Self
    }
}

/// Derive the mirrored-values key for a `get*` query name:
/// `getSelection` -> `selection`, `getValue` -> `value`.
fn value_style_key(query: &str) -> Option<String> {
    let rest = query.strip_prefix("get")?;
    let mut chars = rest.chars();
    let first = chars.next()?;
    Some(first.to_lowercase().collect::<String>() + chars.as_str())
}

/// Extract the mirrored values map from a `"{instance_id}/values"` payload
/// entry. Accepts `{ "values": {...} }` envelopes (flw/1 `value:changed`
/// shape), the BoundValue shape emitted in workflow element payloads,
/// component-wrapped entries, or a plain map.
fn extract_mirrored_values(entry: &Value) -> Option<flow_like_types::json::Map<String, Value>> {
    if let Some(values) = entry.get("values").and_then(|v| v.as_object()) {
        return Some(values.clone());
    }
    if let Some(values) = entry
        .get("component")
        .and_then(|c| c.get("values"))
        .and_then(|v| v.as_object())
    {
        return Some(values.clone());
    }

    if let Some(bound_value) = entry.get("component").and_then(|c| c.get("value")) {
        if let Some(literal_json) = bound_value.get("literalJson").and_then(Value::as_str)
            && let Ok(Value::Object(values)) =
                flow_like_types::json::from_str::<Value>(literal_json)
        {
            return Some(values);
        }

        // Tolerate hosts that already resolved the BoundValue and placed the
        // literal object directly under component.value.
        if let Some(values) = bound_value.as_object()
            && !values.keys().any(|key| key.starts_with("literal"))
        {
            return Some(values.clone());
        }
    }
    entry.as_object().cloned()
}

fn add_query_arg_pins(node: &mut Node, query: &ContractQuery) -> BTreeSet<String> {
    let mut expected = BTreeSet::new();
    let Some(args_schema) = &query.args_schema else {
        return expected;
    };
    if args_schema.is_null() {
        return expected;
    }

    let properties = args_schema.get("properties").and_then(|p| p.as_object());
    match properties {
        Some(props) => {
            for (key, prop_schema) in props {
                let pin_name = format!("{DYN_ARG_PREFIX}{key}");
                expected.insert(pin_name.clone());
                let shape = json_schema_pin_shape(prop_schema);
                let description = prop_schema
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or(key);

                // Existing pins keep their value and connections but follow the
                // current contract shape.
                if let Some(existing) = node.get_pin_mut_by_name(&pin_name) {
                    existing.friendly_name = key.clone();
                    existing.description = description.to_string();
                    shape.apply(existing);
                    apply_query_arg_schema(existing, prop_schema, false);
                    continue;
                }

                let pin = node.add_input_pin(&pin_name, key, description, shape.data_type.clone());
                shape.apply(pin);
                apply_query_arg_schema(pin, prop_schema, true);
            }
        }
        None => {
            let pin_name = format!("{DYN_ARG_PREFIX}args");
            expected.insert(pin_name.clone());
            let shape = json_schema_pin_shape(args_schema);
            if let Some(existing) = node.get_pin_mut_by_name(&pin_name) {
                existing.friendly_name = "Args".to_string();
                existing.description = "Query arguments (typed)".to_string();
                shape.apply(existing);
                apply_query_arg_schema(existing, args_schema, false);
            } else {
                let pin = node.add_input_pin(
                    &pin_name,
                    "Args",
                    "Query arguments (typed)",
                    shape.data_type.clone(),
                );
                shape.apply(pin);
                apply_query_arg_schema(pin, args_schema, true);
            }
        }
    }
    expected
}

fn apply_query_arg_schema(pin: &mut Pin, schema: &Value, apply_default: bool) {
    let mut options = PinOptions::new();
    let mut has_options = false;

    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let values: Vec<String> = values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if !values.is_empty() {
            options.set_valid_values(values);
            has_options = true;
        }
    }
    if let (Some(min), Some(max)) = (
        schema.get("minimum").and_then(Value::as_f64),
        schema.get("maximum").and_then(Value::as_f64),
    ) {
        options.set_range((min, max));
        has_options = true;
    }
    if let Some(step) = schema.get("multipleOf").and_then(Value::as_f64) {
        options.set_step(step);
        has_options = true;
    }
    pin.options = has_options.then(|| options.build());

    if apply_default && let Some(default) = schema.get("default") {
        pin.set_default_value(Some(default.clone()));
    }
}

fn raw_query_arg_pin_name(node: &Node) -> Option<String> {
    node.pins
        .values()
        .find(|pin| pin.name == format!("{DYN_ARG_PREFIX}args") && pin.friendly_name == "Args")
        .map(|pin| pin.name.clone())
}

fn selected_query_name(node: &Node) -> Option<String> {
    node.get_pin_by_name("query")
        .and_then(|pin| pin.default_value.as_ref())
        .and_then(|bytes| flow_like_types::json::from_slice::<String>(bytes).ok())
        .filter(|value| !value.is_empty())
}

fn reset_value_pin(node: &mut Node) {
    if let Some(value_pin) = node.get_pin_mut_by_name("value") {
        value_pin.data_type = VariableType::Generic;
        value_pin.value_type = ValueType::Normal;
        value_pin.schema = None;
    }
}

fn reset_query_shape(node: &mut Node, clear_options: bool) {
    remove_stale_prefixed_pins(node, DYN_ARG_PREFIX, &BTreeSet::new());
    reset_value_pin(node);
    if clear_options && let Some(query_pin) = node.get_pin_mut_by_name("query") {
        query_pin.options = None;
    }
    node.friendly_name = "Query Widget".to_string();
}

/// Apply one frozen contract to the node. This is deliberately independent of
/// package lookup so connection-carried metadata and registry resolution share
/// exactly the same dropdown, argument-pin, and result-pin behavior.
fn configure_query_contract(node: &mut Node, widget_name: &str, contract: &WidgetContract) {
    let widget_name = if widget_name.is_empty() {
        contract.id.as_str()
    } else {
        widget_name
    };
    let query_names: Vec<String> = contract.queries.keys().cloned().collect();
    if let Some(query_pin) = node.get_pin_mut_by_name("query") {
        query_pin.set_options(
            PinOptions::new()
                .set_valid_values(query_names.clone())
                .build(),
        );
    }

    let selected_query = selected_query_name(node);
    let Some(query_name) = selected_query.as_deref() else {
        remove_stale_prefixed_pins(node, DYN_ARG_PREFIX, &BTreeSet::new());
        reset_value_pin(node);
        node.friendly_name = format!("Query {widget_name}");
        if query_names.is_empty() {
            node.error = Some(format!("Widget '{widget_name}' declares no queries."));
        }
        return;
    };

    let Some(query) = contract.queries.get(query_name) else {
        remove_stale_prefixed_pins(node, DYN_ARG_PREFIX, &BTreeSet::new());
        reset_value_pin(node);
        node.friendly_name = format!("Query {widget_name}");
        node.error = Some(format!(
            "Query '{}' is not defined on widget '{}'. Available: [{}]",
            query_name,
            widget_name,
            query_names.join(", ")
        ));
        return;
    };

    node.friendly_name = format!("Query {widget_name} ({query_name})");
    let expected = add_query_arg_pins(node, query);
    remove_stale_prefixed_pins(node, DYN_ARG_PREFIX, &expected);

    if let Some(value_pin) = node.get_pin_mut_by_name("value") {
        match &query.result_schema {
            Some(schema) if !schema.is_null() => json_schema_pin_shape(schema).apply(value_pin),
            _ => {
                value_pin.data_type = VariableType::Generic;
                value_pin.value_type = ValueType::Normal;
                value_pin.schema = None;
            }
        }
    }
}

/// Instantiate Widget refs carry `instanceId` at the top level, while Get
/// Element returns a SurfaceComponent with it nested under `component`.
fn extract_widget_instance_id(value: &Value) -> Option<String> {
    value
        .get("instanceId")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("component")
                .and_then(|component| component.get("instanceId"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| extract_element_id(value))
}

#[async_trait]
impl NodeLogic for WidgetQuery {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_widget_query",
            "Query Widget",
            "Reads a typed query result from a package widget instance. Connect Element Ref from Instantiate Widget, or Element from Get Element for a widget placed in the visual builder, then select a contract query.",
            "UI/Container",
        );
        node.set_flowscript_name("ui", "widgetQuery");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element Ref",
            "Package widget reference from Instantiate Widget, or a visual-builder widget from Get Element",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.add_input_pin(
            "query",
            "Query",
            "Contract query to run on the widget instance",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "value",
            "Value",
            "The query result, typed by the contract's result schema",
            VariableType::Generic,
        );

        node.set_long_running(true);
        // No static pin migration: keeping v1 preserves existing dynamic pin
        // IDs, values, and connections while on_update refreshes their shape.
        node.set_version(1);

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        if !pin_connected(node, "element_ref") {
            reset_query_shape(node, true);
            return;
        }

        // Preferred path: Instantiate Widget and Get Element freeze the exact
        // contract onto their reference pin. This works across hosts, reroutes,
        // and layer bridges without a second registry lookup.
        if let Some(metadata) = connected_widget_contract(board, node, "element_ref") {
            configure_query_contract(node, &metadata.widget_name, &metadata.contract);
            return;
        }

        // Backward-compatible path for boards created before reference pins
        // carried metadata.
        let Some(selector) = trace_widget_selector(board, node, "element_ref") else {
            // This is an authoritative non-widget/unsupported source, not a
            // transient registry outage. Do not leave a previous widget's
            // query contract visible after rewiring Get Element.
            reset_query_shape(node, true);
            node.error = Some(
                "Could not discover this widget's contract. Connect Element Ref from Instantiate Widget, or Element from Get Element for a visual-builder package widget."
                    .to_string(),
            );
            return;
        };

        if decode_package_widget_ref(&selector).is_none() {
            reset_query_shape(node, true);
            node.error =
                Some("Query Widget only supports package widgets with a typed contract.".into());
            return;
        }

        let provider = WidgetProvider::from_board(board).await;
        let Some(ResolvedWidget::Package(entry)) = provider.resolve(&selector) else {
            // Do not remove a previously resolved shape during a transient
            // registry outage; connections and configured values must survive.
            node.error = Some(format!(
                "The contract for package widget '{}' is unavailable. Refresh after the package registry is ready.",
                selector
            ));
            return;
        };
        let contract = match entry.parsed_contract() {
            Ok(contract) => contract,
            Err(e) => {
                node.error = Some(e.to_string());
                return;
            }
        };
        configure_query_contract(node, &entry.name, &contract);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_widget_instance_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected a package widget from Instantiate Widget or Get Element"
            )
        })?;

        let query: String = context.evaluate_pin("query").await?;
        if query.is_empty() {
            return Err(flow_like_types::anyhow!(
                "No query selected on Query Widget for instance '{}'",
                element_id
            ));
        }

        // Object schemas become one pin per property. Scalar/array schemas use
        // the single friendly "Args" pin and must be sent as the raw value,
        // not wrapped as { "args": value }.
        let raw_arg_pin = {
            let node = context.node.node.lock().await;
            raw_query_arg_pin_name(&node)
        };
        let args = if let Some(pin_name) = raw_arg_pin {
            context
                .evaluate_pin::<Value>(&pin_name)
                .await
                .ok()
                .filter(|value| !value.is_null())
        } else {
            let args = super::micro_widget_utils::collect_prefixed_pin_values(
                context,
                super::micro_widget_utils::DYN_ARG_PREFIX,
            )
            .await;
            (!args.is_empty()).then_some(Value::Object(args))
        };

        // Live round-trip first: a rendered surface answers via the frontend
        // request channel. Falls back to the value:changed mirror when no
        // surface is live (e.g. headless runs, closed page).
        let live = context
            .query_widget(
                &element_id,
                &query,
                args,
                std::time::Duration::from_millis(LIVE_QUERY_TIMEOUT_MS),
            )
            .await;
        match live {
            Ok(envelope) => {
                let ok = envelope.get("ok").and_then(Value::as_bool).unwrap_or(false);
                if ok {
                    let value = envelope.get("value").cloned().unwrap_or(Value::Null);
                    context
                        .get_pin_by_name("value")
                        .await?
                        .set_value(value)
                        .await;
                    context.activate_exec_pin("exec_out").await?;
                    return Ok(());
                }
                let error = envelope
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("widget returned an error without a message");
                return Err(flow_like_types::anyhow!(
                    "Widget query '{}' on instance '{}' failed in the widget: {}",
                    query,
                    element_id,
                    error
                ));
            }
            Err(err) => {
                context.log_message(
                    &format!(
                        "Live widget query '{}' on '{}' got no response ({}); falling back to mirrored values",
                        query, element_id, err
                    ),
                    LogLevel::Debug,
                );
            }
        }

        let mirror_key = format!("{}/values", element_id);
        let mirror = context
            .read_element(&mirror_key)
            .await?
            .map(|(_, value)| value);

        if let Some(values) = mirror.as_ref().and_then(extract_mirrored_values) {
            let hit = values
                .get(&query)
                .or_else(|| value_style_key(&query).and_then(|key| values.get(&key)));

            if let Some(result) = hit {
                context.log_message(
                    &format!(
                        "Query '{}' on '{}' resolved from mirrored widget values",
                        query, element_id
                    ),
                    LogLevel::Debug,
                );
                context
                    .get_pin_by_name("value")
                    .await?
                    .set_value(result.clone())
                    .await;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }

            let available: Vec<&str> = values.keys().map(|k| k.as_str()).collect();
            return Err(flow_like_types::anyhow!(
                "Widget query '{}' on instance '{}' failed: no live surface answered within {}ms and no mirrored value matches the query. Mirrored keys: [{}]",
                query,
                element_id,
                LIVE_QUERY_TIMEOUT_MS,
                available.join(", ")
            ));
        }

        Err(flow_like_types::anyhow!(
            "Widget query '{}' on instance '{}' failed: no live surface answered within {}ms and no mirrored values were found at '{}'. Ensure the widget's surface is open, or that the widget publishes its state via value:changed.",
            query,
            element_id,
            LIVE_QUERY_TIMEOUT_MS,
            mirror_key
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a2ui::micro_widget_utils::set_widget_ref_metadata;
    use flow_like_storage::object_store::path::Path;
    use flow_like_types::json::json;

    fn sales_contract() -> WidgetContract {
        flow_like_types::json::from_value(json!({
            "contractVersion": 1,
            "id": "sales-chart",
            "queries": {
                "getSelection": {
                    "argsSchema": null,
                    "resultSchema": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "selected": { "type": "boolean" }
                        }
                    }
                },
                "getSeries": {
                    "argsSchema": {
                        "type": "object",
                        "properties": {
                            "top": {
                                "type": "integer",
                                "description": "Highest rows",
                                "minimum": 1,
                                "maximum": 100,
                                "multipleOf": 1,
                                "default": 10
                            }
                        }
                    },
                    "resultSchema": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string" },
                                "value": { "type": "number" }
                            }
                        }
                    }
                },
                "getTotal": {
                    "argsSchema": null,
                    "resultSchema": { "type": "number" }
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn test_value_style_key() {
        assert_eq!(value_style_key("getSelection"), Some("selection".into()));
        assert_eq!(value_style_key("getValue"), Some("value".into()));
        assert_eq!(value_style_key("selection"), None);
        assert_eq!(value_style_key("get"), None);
    }

    #[test]
    fn test_extract_mirrored_values() {
        let envelope = json!({ "values": { "value": "42" } });
        assert!(
            extract_mirrored_values(&envelope)
                .unwrap()
                .contains_key("value")
        );

        let component = json!({ "component": { "values": { "selection": [1, 2] } } });
        assert!(
            extract_mirrored_values(&component)
                .unwrap()
                .contains_key("selection")
        );

        let workflow_payload = json!({
            "id": "values",
            "component": {
                "value": {
                    "literalJson": "{\"getTotal\":42,\"selection\":{\"selected\":true}}"
                }
            }
        });
        let decoded = extract_mirrored_values(&workflow_payload).unwrap();
        assert_eq!(decoded.get("getTotal"), Some(&json!(42)));
        assert_eq!(
            decoded
                .get("selection")
                .and_then(|value| value.get("selected")),
            Some(&json!(true))
        );

        let resolved_payload = json!({
            "component": { "value": { "getTotal": 43 } }
        });
        assert_eq!(
            extract_mirrored_values(&resolved_payload)
                .unwrap()
                .get("getTotal"),
            Some(&json!(43))
        );

        let plain = json!({ "value": "42" });
        assert!(
            extract_mirrored_values(&plain)
                .unwrap()
                .contains_key("value")
        );

        assert!(extract_mirrored_values(&json!("string")).is_none());
    }

    #[test]
    fn test_add_query_arg_pins_from_object_schema() {
        let query = ContractQuery {
            args_schema: Some(json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max rows" },
                    "filter": { "type": "string" }
                }
            })),
            result_schema: None,
            description: None,
        };

        let mut node = Node::new("test", "Test", "Test", "Test");
        let expected = add_query_arg_pins(&mut node, &query);

        assert_eq!(
            expected,
            BTreeSet::from(["dyn_arg_filter".to_string(), "dyn_arg_limit".to_string()])
        );
        let limit = node.get_pin_by_name("dyn_arg_limit").unwrap();
        assert_eq!(limit.data_type, VariableType::Integer);
        assert_eq!(limit.description, "Max rows");
        assert_eq!(
            node.get_pin_by_name("dyn_arg_filter").unwrap().data_type,
            VariableType::String
        );
    }

    #[test]
    fn test_add_query_arg_pins_without_schema() {
        let mut node = Node::new("test", "Test", "Test", "Test");
        assert!(add_query_arg_pins(&mut node, &ContractQuery::default()).is_empty());

        let null_schema = ContractQuery {
            args_schema: Some(Value::Null),
            result_schema: None,
            description: None,
        };
        assert!(add_query_arg_pins(&mut node, &null_schema).is_empty());

        let scalar = ContractQuery {
            args_schema: Some(json!({ "type": "string" })),
            result_schema: None,
            description: None,
        };
        let expected = add_query_arg_pins(&mut node, &scalar);
        assert_eq!(expected, BTreeSet::from(["dyn_arg_args".to_string()]));
        assert_eq!(
            node.get_pin_by_name("dyn_arg_args").unwrap().data_type,
            VariableType::String
        );
    }

    #[test]
    fn contract_populates_query_dropdown_before_a_query_is_selected() {
        let mut node = WidgetQuery::new().get_node();
        configure_query_contract(&mut node, "Sales Chart", &sales_contract());

        let query = node.get_pin_by_name("query").unwrap();
        assert_eq!(
            query.options.as_ref().unwrap().valid_values,
            Some(vec![
                "getSelection".to_string(),
                "getSeries".to_string(),
                "getTotal".to_string(),
            ])
        );
        assert!(node.get_pin_by_name("dyn_arg_top").is_none());
        assert_eq!(node.friendly_name, "Query Sales Chart");
    }

    #[flow_like_types::tokio::test]
    async fn connected_contract_metadata_populates_dropdown_without_a_registry() {
        let contract = sales_contract();
        let contract_value = flow_like_types::json::to_value(&contract).unwrap();
        let mut source = Node::new("source", "Source", "Source", "Test");
        let source_pin = source.add_output_pin(
            "element_ref",
            "Element Ref",
            "Widget reference",
            VariableType::Struct,
        );
        set_widget_ref_metadata(
            source_pin,
            "pkg:com.example.sales/sales-chart",
            "Sales Chart",
            &contract_value,
        );
        let source_pin_id = source_pin.id.clone();

        let mut query_node = WidgetQuery::new().get_node();
        query_node
            .get_pin_mut_by_name("element_ref")
            .unwrap()
            .depends_on
            .insert(source_pin_id);

        let mut board = Board::new_detached(Some("query-metadata-test".into()), Path::default());
        board.nodes.insert(source.id.clone(), source);

        WidgetQuery::new().on_update(&mut query_node, &board).await;

        let options = query_node
            .get_pin_by_name("query")
            .and_then(|pin| pin.options.as_ref())
            .and_then(|options| options.valid_values.as_ref())
            .cloned();
        assert_eq!(
            options,
            Some(vec![
                "getSelection".to_string(),
                "getSeries".to_string(),
                "getTotal".to_string(),
            ])
        );
        assert_eq!(query_node.error, None);
    }

    #[flow_like_types::tokio::test]
    async fn rewiring_to_a_non_widget_clears_the_previous_query_contract() {
        let mut query_node = WidgetQuery::new().get_node();
        query_node
            .get_pin_mut_by_name("query")
            .unwrap()
            .set_default_value(Some(json!("getSeries")));
        configure_query_contract(&mut query_node, "Sales Chart", &sales_contract());
        assert!(query_node.get_pin_by_name("dyn_arg_top").is_some());

        let mut source = Node::new("source", "Source", "Source", "Test");
        let source_pin_id = source
            .add_output_pin(
                "element",
                "Element",
                "A normal builder component",
                VariableType::Struct,
            )
            .id
            .clone();
        query_node
            .get_pin_mut_by_name("element_ref")
            .unwrap()
            .depends_on
            .insert(source_pin_id);

        let mut board = Board::new_detached(Some("query-non-widget-test".into()), Path::default());
        board.nodes.insert(source.id.clone(), source);
        WidgetQuery::new().on_update(&mut query_node, &board).await;

        assert!(query_node.get_pin_by_name("dyn_arg_top").is_none());
        assert!(
            query_node
                .get_pin_by_name("query")
                .unwrap()
                .options
                .is_none()
        );
        let value = query_node.get_pin_by_name("value").unwrap();
        assert_eq!(value.data_type, VariableType::Generic);
        assert_eq!(value.value_type, ValueType::Normal);
        assert!(query_node.error.is_some());
    }

    #[test]
    fn selected_query_creates_stable_argument_and_typed_result_pins() {
        let mut node = WidgetQuery::new().get_node();
        node.get_pin_mut_by_name("query")
            .unwrap()
            .set_default_value(Some(json!("getSeries")));

        configure_query_contract(&mut node, "Sales Chart", &sales_contract());
        let argument = node.get_pin_by_name("dyn_arg_top").unwrap();
        let argument_id = argument.id.clone();
        assert_eq!(argument.data_type, VariableType::Integer);
        assert_eq!(argument.description, "Highest rows");
        assert_eq!(argument.options.as_ref().unwrap().range, Some((1.0, 100.0)));
        assert_eq!(argument.options.as_ref().unwrap().step, Some(1.0));
        assert_eq!(argument.default_value.as_deref(), Some(b"10".as_slice()));

        let value = node.get_pin_by_name("value").unwrap();
        assert_eq!(value.data_type, VariableType::Struct);
        assert_eq!(value.value_type, ValueType::Array);
        assert!(value.schema.as_deref().unwrap().contains("label"));

        configure_query_contract(&mut node, "Sales Chart", &sales_contract());
        assert_eq!(node.get_pin_by_name("dyn_arg_top").unwrap().id, argument_id);
    }

    #[test]
    fn switching_queries_removes_stale_arguments_and_retypes_value() {
        let mut node = WidgetQuery::new().get_node();
        node.get_pin_mut_by_name("query")
            .unwrap()
            .set_default_value(Some(json!("getSeries")));
        configure_query_contract(&mut node, "Sales Chart", &sales_contract());
        assert!(node.get_pin_by_name("dyn_arg_top").is_some());

        node.get_pin_mut_by_name("query")
            .unwrap()
            .set_default_value(Some(json!("getTotal")));
        configure_query_contract(&mut node, "Sales Chart", &sales_contract());

        assert!(node.get_pin_by_name("dyn_arg_top").is_none());
        let value = node.get_pin_by_name("value").unwrap();
        assert_eq!(value.data_type, VariableType::Float);
        assert_eq!(value.value_type, ValueType::Normal);
        assert_eq!(value.schema, None);
    }

    #[test]
    fn visual_builder_element_uses_nested_widget_instance_id() {
        assert_eq!(
            extract_widget_instance_id(&json!({
                "id": "microWidgetInstance-micro-sales-chart-1",
                "__element_id": "page/microWidgetInstance-micro-sales-chart-1",
                "component": {
                    "type": "microWidgetInstance",
                    "instanceId": "micro-sales-chart-1"
                }
            })),
            Some("micro-sales-chart-1".to_string())
        );
        assert_eq!(
            extract_widget_instance_id(&json!({
                "id": "instantiated-1",
                "instanceId": "instantiated-1"
            })),
            Some("instantiated-1".to_string())
        );
    }

    #[test]
    fn scalar_or_array_query_args_are_marked_for_raw_runtime_delivery() {
        let contract: WidgetContract = flow_like_types::json::from_value(json!({
            "contractVersion": 1,
            "id": "search",
            "queries": {
                "raw": {
                    "argsSchema": { "type": "array", "items": { "type": "string" } }
                },
                "object": {
                    "argsSchema": {
                        "type": "object",
                        "properties": { "args": { "type": "string" } }
                    }
                }
            }
        }))
        .unwrap();

        let mut node = WidgetQuery::new().get_node();
        node.get_pin_mut_by_name("query")
            .unwrap()
            .set_default_value(Some(json!("raw")));
        configure_query_contract(&mut node, "Search", &contract);
        assert_eq!(
            raw_query_arg_pin_name(&node).as_deref(),
            Some("dyn_arg_args")
        );

        node.get_pin_mut_by_name("query")
            .unwrap()
            .set_default_value(Some(json!("object")));
        configure_query_contract(&mut node, "Search", &contract);
        assert_eq!(raw_query_arg_pin_name(&node), None);
        assert_eq!(
            node.get_pin_by_name("dyn_arg_args").unwrap().friendly_name,
            "args"
        );
    }
}
