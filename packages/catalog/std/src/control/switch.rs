use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{Pin, PinType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

const CASE_PREFIX: &str = "case_";

/// Pin names have to survive a case being renamed or removed, so they are derived
/// from the case value itself rather than its position. Values that sanitise to
/// the same name get an index suffix, which keeps the derivation deterministic.
fn case_pin_names(cases: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(cases.len());

    for case in cases {
        let sanitized: String = case
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();

        let base = format!("{CASE_PREFIX}{}", sanitized.trim_matches('_'));
        let mut name = base.clone();
        let mut suffix = 2;
        while names.contains(&name) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        names.push(name);
    }

    names
}

fn parse_cases(literal: &str) -> Vec<String> {
    literal
        .split(',')
        .map(|case| case.trim().to_string())
        .filter(|case| !case.is_empty())
        .collect()
}

/// The cases a wired dropdown already declares. A pin that states its valid values
/// is the authoritative list — typing them a second time into the literal is how
/// switches drift out of sync with the enum they switch on.
fn cases_from_source(node: &Node, board: &Board) -> Option<Vec<String>> {
    let value_pin = node.get_pin_by_name("value")?;
    let source = value_pin.depends_on.iter().next()?;
    let source = board.get_pin_by_id(source)?;
    let values = source.options.as_ref()?.valid_values.as_ref()?;

    if values.is_empty() {
        return None;
    }

    Some(values.clone())
}

/// A case is always a scalar, so arrays and objects never match one — switching
/// on a whole struct is a comparison the user did not mean to write.
fn matches_case(value: &Value, case: &str) -> bool {
    match value {
        Value::String(text) => text == case,
        Value::Null => case.eq_ignore_ascii_case("null"),
        Value::Bool(flag) => case.parse::<bool>().is_ok_and(|expected| expected == *flag),
        Value::Number(number) => number.as_f64() == case.parse::<f64>().ok(),
        _ => false,
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SwitchNode {}

impl SwitchNode {
    pub fn new() -> Self {
        SwitchNode {}
    }
}

#[async_trait]
impl NodeLogic for SwitchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_switch",
            "Switch",
            "Sends the flow down one branch per value. Wire a dropdown pin and the cases fill in by themselves, otherwise list them below",
            "Control/Flow",
        );
        node.add_icon("/flow/icons/split.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("exec_in", "In", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "value",
            "Value",
            "The value to switch on",
            VariableType::Generic,
        );
        node.add_input_pin(
            "cases",
            "Cases",
            "Comma separated list of values to branch on. Ignored while the wired pin declares its own values",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "default",
            "Default",
            "Taken when no case matched",
            VariableType::Execution,
        );
        node.add_output_pin(
            "matched_case",
            "Matched Case",
            "The case that was taken, empty when the default ran",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: Value = context.evaluate_pin("value").await?;

        // The case list is rebuilt from the pins themselves: whatever `on_update`
        // minted is the truth at runtime, whether it came from an enum or a literal.
        // The friendly name carries the case value, so it lives on the node, not on
        // the execution pin.
        let cases: Vec<(String, String)> = {
            let node = context.node.node.lock().await;
            let mut found: Vec<(u16, String, String)> = node
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Output && pin.name.starts_with(CASE_PREFIX))
                .map(|pin| (pin.index, pin.name.clone(), pin.friendly_name.clone()))
                .collect();
            found.sort_by_key(|(index, _, _)| *index);
            found
                .into_iter()
                .map(|(_, name, friendly)| (name, friendly))
                .collect()
        };

        context.deactivate_exec_pin("default").await?;
        for (name, _) in cases.iter() {
            context.deactivate_exec_pin(name).await?;
        }

        let hit = cases
            .iter()
            .find(|(_, friendly)| matches_case(&value, friendly));

        match hit {
            Some((name, friendly)) => {
                context
                    .set_pin_value("matched_case", json!(friendly))
                    .await?;
                context.activate_exec_pin(name).await?;
            }
            None => {
                context.set_pin_value("matched_case", json!("")).await?;
                context.activate_exec_pin("default").await?;
            }
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);

        let cases = match cases_from_source(node, board) {
            Some(cases) => cases,
            None => {
                let literal = node
                    .get_pin_by_name("cases")
                    .and_then(|pin| pin.default_value.clone())
                    .and_then(|raw| flow_like_types::json::from_slice::<String>(&raw).ok())
                    .unwrap_or_default();
                parse_cases(&literal)
            }
        };

        let names = case_pin_names(&cases);
        let wanted: Vec<(String, String)> = names.into_iter().zip(cases).collect();

        // Adopt by name instead of rebuilding: pin ids are minted randomly, so a
        // rebuild drops every wire on the branch that was already drawn.
        for (name, friendly) in wanted.iter() {
            match node.get_pin_mut_by_name(name) {
                Some(pin) => {
                    pin.friendly_name = friendly.clone();
                }
                None => {
                    node.add_output_pin(
                        name,
                        friendly,
                        "Taken when the value equals this case",
                        VariableType::Execution,
                    );
                }
            }
        }

        let keep: Vec<String> = wanted.into_iter().map(|(name, _)| name).collect();
        node.pins.retain(|_, pin: &mut Pin| {
            if !pin.name.starts_with(CASE_PREFIX) {
                return true;
            }
            keep.contains(&pin.name) || !pin.connected_to.is_empty()
        });
    }
}
