use super::elements::element_utils::extract_element_id;
use super::micro_widget_utils::{
    DYN_INPUT_PREFIX, add_contract_input_pins, collect_prefixed_pin_values,
    expected_contract_input_pin_names, pin_connected, remove_stale_prefixed_pins,
    trace_widget_selector,
};
use flow_like::a2ui::micro_widget::{ResolvedWidget, WidgetProvider, decode_package_widget_ref};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::collections::BTreeSet;

/// Sends a typed props patch to a package (micro) widget instance. The
/// contract is resolved by tracing the connected element ref back to the
/// producing Instantiate Widget node, which generates one optional
/// `dyn_in_*` pin per contract input. Only pins that are connected or
/// explicitly set are included in the patch.
#[crate::register_node]
#[derive(Default)]
pub struct WidgetUpdateInputs;

impl WidgetUpdateInputs {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for WidgetUpdateInputs {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_widget_update_inputs",
            "Update Widget Inputs",
            "Sends a typed input patch to a package widget instance. Connect the Element Ref from Instantiate Widget to generate one optional pin per contract input; only set pins are included in the patch.",
            "UI/Container",
        );
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
            "Element reference of a package widget instance (from Instantiate Widget)",
            VariableType::Struct,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);
        node.set_version(1);

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        if !pin_connected(node, "element_ref") {
            remove_stale_prefixed_pins(node, DYN_INPUT_PREFIX, &BTreeSet::new());
            return;
        }

        let Some(selector) = trace_widget_selector(board, node, "element_ref") else {
            // Connected through something we cannot trace (e.g. a variable):
            // keep existing pins so connections survive board parses.
            return;
        };

        if decode_package_widget_ref(&selector).is_none() {
            remove_stale_prefixed_pins(node, DYN_INPUT_PREFIX, &BTreeSet::new());
            node.error = Some(
                "Update Widget Inputs only supports package widgets. Use the element Set nodes to update declarative widget internals.".to_string(),
            );
            return;
        }

        let provider = WidgetProvider::from_board(board).await;
        match provider.resolve(&selector) {
            Some(ResolvedWidget::Package(entry)) => match entry.parsed_contract() {
                Ok(contract) => {
                    let expected = expected_contract_input_pin_names(&contract);
                    remove_stale_prefixed_pins(node, DYN_INPUT_PREFIX, &expected);
                    node.friendly_name = format!("Update {} Inputs", entry.name);
                    add_contract_input_pins(node, &contract, false);
                }
                Err(e) => {
                    remove_stale_prefixed_pins(node, DYN_INPUT_PREFIX, &BTreeSet::new());
                    node.error = Some(e.to_string());
                }
            },
            _ => {
                // Package currently unresolvable (e.g. package removed from
                // the app): keep pins, the run will surface the error.
            }
        }
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected element ref from Instantiate Widget"
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
