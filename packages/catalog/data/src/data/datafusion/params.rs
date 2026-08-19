//! Query parameter pins for the SQL nodes.
//!
//! A node whose query is a literal derives one input pin per `$placeholder` in that
//! literal — the same shape as `string_format`'s `{token}` pins, so a value flows into a
//! query by wiring it rather than by being concatenated into the SQL text. Values are
//! bound by the DataFusion planner (see
//! [`flow_like_storage::databases::sql_params`]), so a parameter can never widen the
//! statement it sits in.
//!
//! Nodes whose query arrives over a wire cannot have their placeholders read ahead of
//! time, so every node also carries a `params` object pin. That pin is the general
//! channel; the derived pins are the discoverable one.

use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, remove_unwired_pins},
    pin::{Pin, PinType},
    variable::VariableType,
};
use flow_like_ast::to_camel_case;
use flow_like_storage::databases::sql_params::{
    declared_placeholders, param_pin_name, placeholder_from_pin_name, resolve_query_params,
};
use flow_like_types::{Result, Value, json::json};
use std::collections::HashMap;

/// Name of the object pin that supplies parameters by name.
pub const PARAMS_PIN: &str = "params";

const PARAMS_PIN_DESCRIPTION: &str = "Values for the query's $placeholders, as an object keyed by placeholder name without the $ (e.g. {\"customer_id\": 42}). Only needed when the query itself comes from a wire — a literal query derives one pin per placeholder instead. Where both supply the same name, the derived pin wins unless it is empty.";

/// Adds the parameter object pin. Call from `get_node` on every node that binds
/// parameters, so the channel exists even when the query is not a literal.
pub fn add_params_pin(node: &mut Node) {
    node.add_input_pin(
        PARAMS_PIN,
        "Params",
        PARAMS_PIN_DESCRIPTION,
        VariableType::Struct,
    )
    .set_default_value(Some(json!({})));
}

/// Reconciles the node's derived `$placeholder` pins against its query literal.
///
/// Call from `on_update` after clearing `node.error`; this sets `node.error` when the
/// literal cannot be read and leaves the existing pins untouched in that case — a
/// half-typed query must not disconnect wires the user has already made.
///
/// Pins are keyed by placeholder name, not by occurrence: a placeholder repeated in the
/// statement resolves to one pin, bound once at every occurrence.
pub fn sync_param_pins(node: &mut Node, query_pin: &str, board: &Board) {
    let Some(query) = query_literal(node, query_pin) else {
        // What runs is decided at runtime, so the stale literal declares nothing: retire the pins
        // it left behind, since offering inputs the real query never asks for is misleading. A pin
        // that is still wired is the exception — removing it deletes the connection on both ends,
        // with no error anywhere, so it is kept and reported instead.
        let derived: Vec<String> = node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input && placeholder_from_pin_name(&pin.name).is_some()
            })
            .map(|pin| pin.id.clone())
            .collect();
        remove_unwired_pins(node, &derived);
        return;
    };

    let placeholders = match declared_placeholders(&query) {
        Ok(placeholders) => placeholders,
        Err(error) => {
            node.error = Some(error.to_string());
            return;
        }
    };

    if let Some(conflict) = flowscript_name_conflict(&placeholders) {
        node.error = Some(conflict);
        return;
    }

    let expected: Vec<String> = placeholders
        .iter()
        .map(|name| param_pin_name(name))
        .collect();

    // Group by name: an earlier pass could have leaked several pins for one placeholder,
    // and collapsing them here is what keeps repeated passes idempotent.
    let mut existing: HashMap<String, Vec<&Pin>> = HashMap::new();
    for pin in node.pins.values() {
        if pin.pin_type != PinType::Input || placeholder_from_pin_name(&pin.name).is_none() {
            continue;
        }
        existing.entry(pin.name.clone()).or_default().push(pin);
    }

    let mut missing: Vec<(String, String)> = Vec::new();
    let mut stale_ids: Vec<String> = Vec::new();

    for (placeholder, pin_name) in placeholders.iter().zip(&expected) {
        match existing.remove(pin_name) {
            Some(mut matched) => {
                // Keep the oldest pin so its id — and therefore its wires — survive.
                matched.sort_by_key(|pin| (pin.index, pin.id.clone()));
                stale_ids.extend(matched.iter().skip(1).map(|pin| pin.id.clone()));
            }
            None => missing.push((placeholder.clone(), pin_name.clone())),
        }
    }

    stale_ids.extend(
        existing
            .values()
            .flatten()
            .map(|pin| pin.id.clone())
            .collect::<Vec<_>>(),
    );

    remove_unwired_pins(node, &stale_ids);

    for (placeholder, pin_name) in missing {
        node.add_input_pin(
            &pin_name,
            &format!("${placeholder}"),
            &format!("Value bound to the ${placeholder} placeholder in the query"),
            VariableType::Generic,
        );
    }

    for pin_name in &expected {
        let _ = node.match_type(pin_name, board, None, None);
    }
}

/// The query literal the pins are derived from, or `None` when it cannot be read.
///
/// A wired query is `None`, not an empty query: its literal is whatever was last typed and no
/// longer describes what will run, so no pin may be derived from it — but neither may the pins it
/// already has be taken as refuted. `None` means "unknown", and the caller must leave the existing
/// pins, and therefore their connections, exactly as they are.
fn query_literal(node: &Node, query_pin: &str) -> Option<String> {
    let pin = node.get_pin_by_name(query_pin)?;
    if !pin.depends_on.is_empty() {
        return None;
    }
    let bytes = pin.default_value.as_ref()?;
    flow_like_types::json::from_slice::<Value>(bytes)
        .ok()?
        .as_str()
        .map(ToOwned::to_owned)
}

/// FlowScript addresses a pin by its camelCased name, so two placeholders that render to
/// the same argument would be indistinguishable to the reconciler — `$foo_bar` and
/// `$fooBar` both become `paramFooBar`, as do `$1` and `$_1`. Naming the clash is the only
/// safe outcome; silently binding one value to both pins is not.
fn flowscript_name_conflict(placeholders: &[String]) -> Option<String> {
    let mut claimed: HashMap<String, String> = HashMap::new();
    for placeholder in placeholders {
        let argument = to_camel_case(&param_pin_name(placeholder));
        if let Some(previous) = claimed.insert(argument.clone(), placeholder.clone()) {
            return Some(format!(
                "Placeholders ${previous} and ${placeholder} both map to the argument '{argument}'. Rename one of them."
            ));
        }
    }
    None
}

/// The parameter values for `query`, ordered by first appearance of each placeholder.
///
/// Reads the `params` object first, then lets each derived pin override its own entry. A
/// derived pin contributes only when it actually holds a value, so the two channels compose:
/// wiring a pin overrides the object, and leaving a pin empty falls back to it instead of
/// blanking it.
///
/// An unset pin is deliberately NOT treated as an explicit null. Binding null would make
/// `WHERE id = $id` match nothing — a filter that silently returns no rows rather than
/// reporting that a value is missing. Errors instead, naming the placeholder.
pub async fn resolve_params(
    context: &mut ExecutionContext,
    query: &str,
) -> Result<Vec<(String, Value)>> {
    let placeholders = declared_placeholders(query)?;
    if placeholders.is_empty() {
        return Ok(Vec::new());
    }

    let supplied = match context.get_pin_by_name(PARAMS_PIN).await {
        Ok(_) => context
            .evaluate_pin::<Value>(PARAMS_PIN)
            .await
            .unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };

    let mut bag = match supplied {
        Value::Object(map) => map,
        Value::Null => flow_like_types::json::Map::new(),
        _ => {
            return Err(flow_like_types::anyhow!(
                "The Params pin must hold an object keyed by placeholder name"
            ));
        }
    };

    for placeholder in &placeholders {
        let pin_name = param_pin_name(placeholder);
        if context.get_pin_by_name(&pin_name).await.is_err() {
            continue;
        }
        let Ok(value) = context.evaluate_pin::<Value>(&pin_name).await else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        bag.insert(placeholder.clone(), value);
    }

    resolve_query_params(query, &Value::Object(bag))
}

/// The resolved parameters as a JSON object, for the surfaces that bind from an object
/// rather than from a DataFrame (e.g. [`flow_like_storage::databases::graph::GraphStore`]).
pub fn to_object(params: &[(String, Value)]) -> Value {
    Value::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

/// Binds `params` onto a planned DataFrame. A query with no placeholders is left alone, so
/// the binding call is safe to make unconditionally.
pub fn bind(
    df: flow_like_storage::datafusion::dataframe::DataFrame,
    params: &[(String, Value)],
) -> Result<flow_like_storage::datafusion::dataframe::DataFrame> {
    if params.is_empty() {
        return Ok(df);
    }
    let values = flow_like_storage::databases::sql_params::to_param_values(params)?;
    Ok(df.with_param_values(values)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicting_placeholder_arguments_are_reported() {
        let conflict = flowscript_name_conflict(&["foo_bar".to_string(), "fooBar".to_string()])
            .expect("clash");
        assert!(conflict.contains("paramFooBar"), "unexpected: {conflict}");

        let numeric =
            flowscript_name_conflict(&["1".to_string(), "_1".to_string()]).expect("clash");
        assert!(numeric.contains("param1"), "unexpected: {numeric}");
    }

    #[test]
    fn distinct_placeholders_do_not_conflict() {
        assert!(
            flowscript_name_conflict(&[
                "customer_id".to_string(),
                "since".to_string(),
                "1".to_string(),
                "2".to_string(),
            ])
            .is_none()
        );
    }
}
