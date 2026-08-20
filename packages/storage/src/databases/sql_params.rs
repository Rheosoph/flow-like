//! Parameter binding shared by every SQL surface: the DataFusion catalog nodes, the
//! Data Studio query workbench and the graph SQL endpoints.
//!
//! Values reach DataFusion as typed [`ScalarValue`]s and are substituted by the planner
//! into `$name` placeholders, so a parameter can only ever be a literal in the position it
//! appears — it cannot widen the statement around it. The injection boundary is the
//! planner, not string escaping.
//!
//! Placeholders are discovered with the *same* tokenizer DataFusion parses with
//! (`sqlparser` under [`GenericDialect`], which is DataFusion's default
//! `datafusion.sql_parser.dialect`), so what this module calls a parameter and what the
//! planner calls one cannot drift. A hand-rolled scan would drift immediately:
//! `$tag$body$tag$` is a dollar-quoted string literal, not a `$tag` placeholder, and `$`
//! inside string literals, quoted identifiers and comments is not a placeholder either.
//!
//! Placeholders are only legal in *expression* position. Table and column names can never
//! be parameterized, so callers that interpolate identifiers still have to quote or
//! allowlist them — parameters alone do not make a fully caller-authored query safe.

use datafusion::common::{ParamValues, ScalarValue};
use datafusion::sql::sqlparser::dialect::GenericDialect;
use datafusion::sql::sqlparser::tokenizer::{Token, Tokenizer};
use flow_like_types::{Result, Value, anyhow};
use std::collections::HashMap;

use datafusion::arrow::datatypes::DataType;

/// Upper bound on distinct placeholders in one statement. A parameterized query is a
/// hand-written filter, not a generated one; the cap exists so a pathological literal
/// cannot mint an unbounded number of pins on a node.
pub const MAX_QUERY_PARAMS: usize = 64;

/// Prefix of the input pin a SQL node mints for one placeholder.
///
/// The prefix is what keeps a placeholder from colliding with a node's own pins: a query
/// may legitimately contain `$query` or `$session`, and an unprefixed pin of that name
/// would shadow the node's configuration. It also namespaces the FlowScript argument
/// (`paramCustomerId:`), so a reader can tell a bound value from a setting.
pub const PARAM_PIN_PREFIX: &str = "param_";

/// The input pin name carrying the value for `$placeholder`.
///
/// Shared by the nodes that mint these pins and by the FlowScript reconciler that predicts
/// them, so the two cannot disagree about what a placeholder is called.
pub fn param_pin_name(placeholder: &str) -> String {
    format!("{PARAM_PIN_PREFIX}{placeholder}")
}

/// The placeholder a param pin carries a value for, or `None` for any other pin.
pub fn placeholder_from_pin_name(pin_name: &str) -> Option<&str> {
    pin_name
        .strip_prefix(PARAM_PIN_PREFIX)
        .filter(|rest| !rest.is_empty())
}

/// Distinct placeholder names declared by `sql`, without the leading `$`, ordered by
/// first appearance. A placeholder repeated in the statement is reported once — it
/// resolves to one value, bound once.
///
/// Errors when the statement cannot be tokenized (an unterminated literal, typically a
/// half-typed query) or when it uses a placeholder form that cannot be addressed by
/// name. Callers deriving pins should treat an error as "leave the current pins alone"
/// rather than as a reason to drop them.
pub fn declared_placeholders(sql: &str) -> Result<Vec<String>> {
    let dialect = GenericDialect {};
    let tokens = Tokenizer::new(&dialect, sql)
        .tokenize()
        .map_err(|error| anyhow!("Could not read the SQL statement: {error}"))?;

    let mut names: Vec<String> = Vec::new();
    for token in tokens {
        let Token::Placeholder(raw) = token else {
            continue;
        };

        // `?` and `?N` tokenize as placeholders but carry no name, so no value can ever
        // be addressed to them. Reject with the fix rather than letting the planner fail
        // later with "No value found for placeholder".
        let Some(name) = raw.strip_prefix('$') else {
            return Err(anyhow!(
                "Unsupported placeholder '{raw}' in SQL. Use a named placeholder like $customer_id, or a numbered one like $1"
            ));
        };

        if name.is_empty() {
            return Err(anyhow!(
                "Found a bare '$' in SQL. Use a named placeholder like $customer_id, or a numbered one like $1"
            ));
        }

        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }

    if names.len() > MAX_QUERY_PARAMS {
        return Err(anyhow!(
            "SQL declares {} parameters, more than the {} supported in one statement",
            names.len(),
            MAX_QUERY_PARAMS
        ));
    }

    Ok(names)
}

/// The values for exactly the placeholders `sql` declares, ordered by first appearance in
/// the statement.
///
/// `supplied` is a JSON object keyed by placeholder name *without* the `$`. Extra keys are
/// ignored — a caller may hold one parameter bag for a family of queries — but a declared
/// placeholder with no value is an error, because binding it to null silently would turn
/// an intended filter into one that matches nothing.
///
/// The stable ordering is what lets a caller fingerprint the resolved parameters (e.g. for
/// a result cache key) without the hash depending on JSON key order.
pub fn resolve_query_params(sql: &str, supplied: &Value) -> Result<Vec<(String, Value)>> {
    resolve_declared("SQL", declared_placeholders(sql)?, supplied)
}

/// Pairs already-discovered placeholders with their values, preserving the order they were
/// declared in.
///
/// Shared with [`super::lance_filter_params`], which discovers its placeholders with a
/// different dialect but owes the caller the same contract: every declared name resolved, or
/// an error naming the ones that were not. `subject` names the thing that declared them, so
/// the message reads as the surface the author is looking at.
pub fn resolve_declared(
    subject: &str,
    declared: Vec<String>,
    supplied: &Value,
) -> Result<Vec<(String, Value)>> {
    if declared.is_empty() {
        return Ok(Vec::new());
    }

    let supplied = as_param_object(supplied)?;

    let mut resolved = Vec::with_capacity(declared.len());
    let mut missing: Vec<String> = Vec::new();
    for name in declared {
        match supplied.and_then(|map| map.get(&name)) {
            Some(value) => resolved.push((name, value.clone())),
            None => missing.push(format!("${name}")),
        }
    }

    if !missing.is_empty() {
        return Err(anyhow!(
            "{subject} declares {} with no value supplied. Connect the matching parameter pin, or supply the name in the parameters object.",
            missing.join(", ")
        ));
    }

    Ok(resolved)
}

/// DataFusion named parameter values for an already-resolved parameter list.
pub fn to_param_values(params: &[(String, Value)]) -> Result<ParamValues> {
    let mut values: HashMap<String, ScalarValue> = HashMap::with_capacity(params.len());
    for (name, value) in params {
        let scalar = json_to_scalar(value)
            .map_err(|error| anyhow!("Parameter '{}' cannot be used in SQL: {}", name, error))?;
        values.insert(name.clone(), scalar);
    }
    Ok(ParamValues::from(values))
}

/// Coerces a JSON parameter map into DataFusion named parameter values. Types are
/// inferred from the supplied values, so binding never interpolates text into the
/// SQL — placeholders are resolved by the planner via `$name`.
///
/// Unlike [`resolve_query_params`] this binds whatever is supplied without consulting the
/// statement, so an unused parameter is not an error and a missing one surfaces later as a
/// planner error.
pub fn bind_params(params: &Value) -> Result<ParamValues> {
    let map = as_param_object(params)?;

    let mut values: HashMap<String, ScalarValue> = HashMap::new();
    for (name, value) in map.into_iter().flatten() {
        let scalar =
            json_to_scalar(value).map_err(|error| anyhow!("Parameter '{}': {}", name, error))?;
        values.insert(name.clone(), scalar);
    }
    Ok(ParamValues::from(values))
}

/// A parameter bag is a JSON object; null stands for "no parameters" so an unset pin or
/// omitted request field does not have to be special-cased by every caller.
fn as_param_object(params: &Value) -> Result<Option<&flow_like_types::json::Map<String, Value>>> {
    match params {
        Value::Object(map) => Ok(Some(map)),
        Value::Null => Ok(None),
        _ => Err(anyhow!("Query parameters must be a JSON object")),
    }
}

fn json_to_scalar(value: &Value) -> Result<ScalarValue> {
    Ok(match value {
        Value::Null => ScalarValue::Null,
        Value::Bool(value) => ScalarValue::Boolean(Some(*value)),
        Value::String(value) => ScalarValue::Utf8(Some(value.clone())),
        Value::Number(_) => json_number_to_scalar(value)?,
        Value::Array(items) => json_array_to_scalar(items)?,
        Value::Object(_) => {
            return Err(anyhow!(
                "object parameters are not supported; pass the individual fields as separate parameters"
            ));
        }
    })
}

fn json_number_to_scalar(value: &Value) -> Result<ScalarValue> {
    let Value::Number(number) = value else {
        return Err(anyhow!("expected a number"));
    };
    Ok(if let Some(int) = number.as_i64() {
        ScalarValue::Int64(Some(int))
    } else if let Some(uint) = number.as_u64() {
        ScalarValue::UInt64(Some(uint))
    } else if let Some(float) = number.as_f64() {
        ScalarValue::Float64(Some(float))
    } else {
        return Err(anyhow!("unsupported numeric parameter"));
    })
}

/// A JSON array becomes a single-row Arrow list, which is what makes a set filter
/// expressible without string building: `array_has($ids, id)` replaces an `IN (…)` list
/// pasted together from user input.
///
/// Elements are unified to one Arrow type up front. This is not cosmetic —
/// `ScalarValue::new_list_nullable` panics on a heterogeneous element iterator, so a
/// mixed array has to be rejected here rather than reaching Arrow.
fn json_array_to_scalar(items: &[Value]) -> Result<ScalarValue> {
    let element_type = unified_element_type(items)?;
    let mut elements = Vec::with_capacity(items.len());
    for item in items {
        elements.push(coerce_element(item, &element_type)?);
    }
    Ok(ScalarValue::List(ScalarValue::new_list_nullable(
        &elements,
        &element_type,
    )))
}

/// The single Arrow type every element of an array parameter is bound as. Integers stay
/// exact where they can (`Int64`), widen to `Float64` only when the array actually mixes
/// in a fractional or out-of-range value.
fn unified_element_type(items: &[Value]) -> Result<DataType> {
    let mut saw_bool = false;
    let mut saw_string = false;
    let mut saw_integer = false;
    let mut saw_float = false;

    for item in items {
        match item {
            Value::Null => {}
            Value::Bool(_) => saw_bool = true,
            Value::String(_) => saw_string = true,
            Value::Number(number) => {
                if number.as_i64().is_some() {
                    saw_integer = true;
                } else {
                    saw_float = true;
                }
            }
            Value::Array(_) | Value::Object(_) => {
                return Err(anyhow!(
                    "nested arrays and objects are not supported inside an array parameter"
                ));
            }
        }
    }

    let kinds = u8::from(saw_bool) + u8::from(saw_string) + u8::from(saw_integer || saw_float);
    if kinds > 1 {
        return Err(anyhow!(
            "array parameters must hold a single type; this one mixes types"
        ));
    }

    Ok(if saw_bool {
        DataType::Boolean
    } else if saw_string {
        DataType::Utf8
    } else if saw_float {
        DataType::Float64
    } else if saw_integer {
        DataType::Int64
    } else {
        // Every element is null (or the array is empty). Utf8 is arbitrary but has to be
        // *something*: an Arrow list always carries an element type.
        DataType::Utf8
    })
}

fn coerce_element(item: &Value, element_type: &DataType) -> Result<ScalarValue> {
    Ok(match (item, element_type) {
        (Value::Null, DataType::Boolean) => ScalarValue::Boolean(None),
        (Value::Null, DataType::Utf8) => ScalarValue::Utf8(None),
        (Value::Null, DataType::Int64) => ScalarValue::Int64(None),
        (Value::Null, DataType::Float64) => ScalarValue::Float64(None),
        (Value::Bool(value), _) => ScalarValue::Boolean(Some(*value)),
        (Value::String(value), _) => ScalarValue::Utf8(Some(value.clone())),
        (Value::Number(number), DataType::Float64) => ScalarValue::Float64(Some(
            number
                .as_f64()
                .ok_or_else(|| anyhow!("unsupported numeric array element"))?,
        )),
        (Value::Number(number), _) => ScalarValue::Int64(Some(
            number
                .as_i64()
                .ok_or_else(|| anyhow!("unsupported numeric array element"))?,
        )),
        _ => return Err(anyhow!("unsupported array element")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    fn placeholders(sql: &str) -> Vec<String> {
        declared_placeholders(sql).expect("tokenizes")
    }

    #[test]
    fn finds_named_and_numbered_placeholders_in_order() {
        assert_eq!(
            placeholders("SELECT * FROM t WHERE a = $name AND b > $1"),
            vec!["name".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn repeated_placeholder_is_reported_once() {
        assert_eq!(
            placeholders("SELECT * FROM t WHERE a = $q OR b = $q OR c = $q"),
            vec!["q".to_string()]
        );
    }

    #[test]
    fn ignores_dollars_that_are_not_placeholders() {
        // A `$` inside a string literal, a quoted identifier or a comment is data, not a
        // parameter. Deriving pins from a naive scan would invent all four of these.
        assert!(placeholders("SELECT '$5.00' AS price FROM t").is_empty());
        assert!(placeholders("SELECT \"$col\" FROM t").is_empty());
        assert!(placeholders("SELECT 1 -- $nope\n FROM t").is_empty());
        assert!(placeholders("SELECT 1 /* $nope */ FROM t").is_empty());
    }

    #[test]
    fn dollar_quoted_string_is_not_a_placeholder() {
        // `$tag$…$tag$` is a string literal under the generic dialect. A regex scan would
        // report `tag` here and mint a pin that the planner never asks for.
        assert!(placeholders("SELECT $tag$body$tag$ FROM t").is_empty());
        assert!(placeholders("SELECT $$body$$ FROM t").is_empty());
    }

    #[test]
    fn question_mark_placeholder_is_rejected_with_guidance() {
        let error = declared_placeholders("SELECT * FROM t WHERE a = ?")
            .expect_err("cannot be addressed by name")
            .to_string();
        assert!(error.contains("$customer_id"), "unexpected error: {error}");
    }

    #[test]
    fn unterminated_literal_is_an_error_not_an_empty_list() {
        // A half-typed query must not read as "this query has no parameters", or a pin
        // deriving caller would drop every param pin mid-edit.
        assert!(declared_placeholders("SELECT * FROM t WHERE a = 'oops").is_err());
    }

    #[test]
    fn resolves_declared_params_in_query_order() {
        let resolved = resolve_query_params(
            "SELECT * FROM t WHERE b = $second AND a = $first",
            &json!({"first": 1, "second": "two", "unused": true}),
        )
        .expect("resolves");

        assert_eq!(
            resolved,
            vec![
                ("second".to_string(), json!("two")),
                ("first".to_string(), json!(1)),
            ]
        );
    }

    #[test]
    fn missing_declared_param_is_an_error() {
        let error =
            resolve_query_params("SELECT * FROM t WHERE a = $a AND b = $b", &json!({"a": 1}))
                .expect_err("b has no value")
                .to_string();
        assert!(error.contains("$b"), "unexpected error: {error}");
    }

    #[test]
    fn no_placeholders_needs_no_parameter_bag() {
        assert!(
            resolve_query_params("SELECT 1", &Value::Null)
                .expect("resolves")
                .is_empty()
        );
        // A non-object bag is only rejected once the query actually declares something,
        // so an unset pin on a parameterless query is not a failure.
        assert!(
            resolve_query_params("SELECT 1", &json!("not an object"))
                .expect("resolves")
                .is_empty()
        );
    }

    #[test]
    fn scalars_map_to_typed_values() {
        assert_eq!(json_to_scalar(&json!(null)).unwrap(), ScalarValue::Null);
        assert_eq!(
            json_to_scalar(&json!(true)).unwrap(),
            ScalarValue::Boolean(Some(true))
        );
        assert_eq!(
            json_to_scalar(&json!("x")).unwrap(),
            ScalarValue::Utf8(Some("x".to_string()))
        );
        assert_eq!(
            json_to_scalar(&json!(7)).unwrap(),
            ScalarValue::Int64(Some(7))
        );
        assert_eq!(
            json_to_scalar(&json!(1.5)).unwrap(),
            ScalarValue::Float64(Some(1.5))
        );
    }

    #[test]
    fn homogeneous_arrays_become_lists() {
        let scalar = json_to_scalar(&json!(["a", "b"])).expect("list");
        assert!(matches!(scalar, ScalarValue::List(_)));
        assert_eq!(
            scalar.data_type(),
            DataType::List(
                datafusion::arrow::datatypes::Field::new_list_field(DataType::Utf8, true).into()
            )
        );

        let ints = json_to_scalar(&json!([1, 2, 3])).expect("list");
        assert_eq!(
            ints.data_type(),
            DataType::List(
                datafusion::arrow::datatypes::Field::new_list_field(DataType::Int64, true).into()
            )
        );
    }

    #[test]
    fn arrays_with_nulls_and_empty_arrays_bind() {
        assert!(matches!(
            json_to_scalar(&json!([1, null, 3])).expect("list"),
            ScalarValue::List(_)
        ));
        assert!(matches!(
            json_to_scalar(&json!([])).expect("list"),
            ScalarValue::List(_)
        ));
        assert!(matches!(
            json_to_scalar(&json!([null])).expect("list"),
            ScalarValue::List(_)
        ));
    }

    #[test]
    fn integers_and_floats_in_one_array_widen_to_float() {
        let scalar = json_to_scalar(&json!([1, 2.5])).expect("list");
        assert_eq!(
            scalar.data_type(),
            DataType::List(
                datafusion::arrow::datatypes::Field::new_list_field(DataType::Float64, true).into()
            )
        );
    }

    #[test]
    fn mixed_type_arrays_are_rejected_rather_than_panicking_arrow() {
        assert!(json_to_scalar(&json!([1, "a"])).is_err());
        assert!(json_to_scalar(&json!([true, 1])).is_err());
        assert!(json_to_scalar(&json!([[1], [2]])).is_err());
        assert!(json_to_scalar(&json!([{"a": 1}])).is_err());
    }

    #[test]
    fn object_parameters_are_rejected() {
        assert!(json_to_scalar(&json!({"a": 1})).is_err());
    }

    #[test]
    fn bind_params_accepts_objects_and_null() {
        assert!(matches!(
            bind_params(&Value::Null).unwrap(),
            ParamValues::Map(map) if map.is_empty()
        ));
        assert!(matches!(
            bind_params(&json!({"a": 1})).unwrap(),
            ParamValues::Map(map) if map.len() == 1
        ));
        assert!(bind_params(&json!([1, 2])).is_err());
    }

    #[test]
    fn to_param_values_reports_the_offending_parameter() {
        let error = to_param_values(&[("ids".to_string(), json!({"a": 1}))])
            .expect_err("objects are not bindable")
            .to_string();
        assert!(error.contains("ids"), "unexpected error: {error}");
    }

    /// Binds `sql` through the same path the nodes use and returns the single-cell result
    /// as a display string, so these tests assert against DataFusion itself rather than
    /// against our own idea of what it accepts.
    async fn run_scalar(sql: &str, params: &Value) -> Result<String> {
        use datafusion::prelude::SessionContext;

        let resolved = resolve_query_params(sql, params)?;
        let ctx = SessionContext::new();
        let batches = ctx
            .sql(sql)
            .await?
            .with_param_values(to_param_values(&resolved)?)?
            .collect()
            .await?;

        let batch = batches.first().ok_or_else(|| anyhow!("no batch"))?;
        let column = batch.column(0);
        Ok(datafusion::arrow::util::display::array_value_to_string(
            column, 0,
        )?)
    }

    #[tokio::test]
    async fn named_parameters_bind_on_a_plain_sql_call() {
        let total = run_scalar(
            "SELECT $left + $right AS total",
            &json!({"left": 2, "right": 40}),
        )
        .await
        .expect("binds");
        assert_eq!(total, "42");
    }

    #[tokio::test]
    async fn numbered_parameters_bind_too() {
        let value = run_scalar("SELECT $1 AS v", &json!({"1": "hello"}))
            .await
            .expect("binds");
        assert_eq!(value, "hello");
    }

    #[tokio::test]
    async fn a_repeated_named_parameter_binds_once_at_every_occurrence() {
        // `ParamValues::Map` does not verify arity, which is what lets one value serve
        // every occurrence. The positional form cannot express this.
        let value = run_scalar("SELECT $n + $n AS v", &json!({"n": 21}))
            .await
            .expect("binds");
        assert_eq!(value, "42");
    }

    #[tokio::test]
    async fn a_string_parameter_cannot_extend_the_statement() {
        // The classic injection payload stays a single string comparison: it is a literal
        // in the plan, never SQL text. A `false` here (not an error, not `true`) is the
        // whole point of binding.
        let value = run_scalar(
            "SELECT 'admin' = $name AS matched",
            &json!({"name": "admin' OR '1'='1"}),
        )
        .await
        .expect("binds");
        assert_eq!(value, "false");
    }

    #[tokio::test]
    async fn a_list_parameter_drives_a_set_filter() {
        // This is the `IN (…)` replacement: without list binding a set filter has to be
        // assembled as text, which is exactly the hole parameters are meant to close.
        let matched = run_scalar(
            "SELECT array_has($ids, 2) AS matched",
            &json!({"ids": [1, 2, 3]}),
        )
        .await
        .expect("binds");
        assert_eq!(matched, "true");

        let missed = run_scalar(
            "SELECT array_has($names, 'zed') AS matched",
            &json!({"names": ["ada", "grace"]}),
        )
        .await
        .expect("binds");
        assert_eq!(missed, "false");
    }

    #[tokio::test]
    async fn a_null_parameter_binds_as_null() {
        let value = run_scalar("SELECT $x IS NULL AS is_null", &json!({"x": null}))
            .await
            .expect("binds");
        assert_eq!(value, "true");
    }

    #[test]
    fn param_pin_names_round_trip() {
        assert_eq!(param_pin_name("customer_id"), "param_customer_id");
        assert_eq!(param_pin_name("1"), "param_1");
        assert_eq!(
            placeholder_from_pin_name("param_customer_id"),
            Some("customer_id")
        );
        assert_eq!(placeholder_from_pin_name("query"), None);
        // A pin literally named `param_` carries no placeholder.
        assert_eq!(placeholder_from_pin_name("param_"), None);
    }

    #[test]
    fn too_many_parameters_is_rejected() {
        let sql = (0..=MAX_QUERY_PARAMS)
            .map(|index| format!("c{index} = $p{index}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        assert!(declared_placeholders(&format!("SELECT * FROM t WHERE {sql}")).is_err());
    }
}
