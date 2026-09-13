use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::Map};

/// Legacy `/use` links prefix app-owned copies of these shell parameters with `_`.
const RESERVED_QUERY_KEYS: &[&str] = &["id", "route", "eventId"];

/// Gets query parameters from the current URL.
///
/// Query parameters are passed via `_query_params` in the workflow payload.
/// For a URL like `/dashboard?tab=settings&page=2`, this would give:
/// `{ "tab": "settings", "page": "2" }`
///
/// Legacy links use `_`-prefixed copies of reserved shell names. App-query
/// envelopes carry `_query_params_format: "app"` beside `_query_params` and
/// preserve names literally, so `id` and `_id` can hold different values.
#[crate::register_node]
#[derive(Default)]
pub struct GetQueryParams;

impl GetQueryParams {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetQueryParams {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_query_params",
            "Get Query Params",
            "Gets query parameters from the current URL",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "getQueryParam");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "param_name",
            "Param Name",
            "The name of the query parameter to get (optional - if empty, returns all params)",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "value",
            "Value",
            "The parameter value (string if param_name specified, object if all params)",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the parameter exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let param_name: String = context.evaluate_pin("param_name").await.unwrap_or_default();

        let payload = context.get_run_payload().await?;
        let (value, exists) = query_param_result(payload.payload.as_ref(), &param_name);

        let value_pin = context.get_pin_by_name("value").await?;
        let exists_pin = context.get_pin_by_name("exists").await?;

        value_pin.set_value(value).await;
        exists_pin.set_value(Value::Bool(exists)).await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

fn query_param_result(payload: Option<&Value>, name: &str) -> (Value, bool) {
    let params = payload
        .and_then(|payload| payload.get("_query_params"))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let app_owned = payload
        .and_then(|payload| payload.get("_query_params_format"))
        .and_then(Value::as_str)
        == Some("app");

    if name.is_empty() {
        return (
            if app_owned {
                params
            } else {
                unwrap_reserved_keys(&params)
            },
            true,
        );
    }

    let value = if app_owned {
        params.get(name)
    } else {
        lookup_param(&params, name)
    };
    match value {
        Some(value) => (value.clone(), true),
        None => (Value::Null, false),
    }
}

fn lookup_param<'a>(query_params: &'a Value, name: &str) -> Option<&'a Value> {
    if RESERVED_QUERY_KEYS.contains(&name) {
        let prefixed = format!("_{name}");
        if let Some(value) = query_params.get(&prefixed) {
            return Some(value);
        }
    }
    query_params.get(name)
}

fn unwrap_reserved_keys(query_params: &Value) -> Value {
    let Some(obj) = query_params.as_object() else {
        return query_params.clone();
    };

    let mut output: Map<String, Value> = Map::with_capacity(obj.len());
    // First pass: copy non-prefixed entries.
    for (key, value) in obj {
        output.insert(key.clone(), value.clone());
    }
    // Second pass: surface `_<reserved>` as `<reserved>`, overriding any
    // framework-supplied value so the workflow sees what the user wrote.
    for reserved in RESERVED_QUERY_KEYS {
        let prefixed = format!("_{reserved}");
        if let Some(value) = obj.get(&prefixed) {
            output.insert((*reserved).to_string(), value.clone());
            output.remove(&prefixed);
        }
    }
    Value::Object(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn lookup_prefers_underscore_prefix_for_reserved_keys() {
        let params = json!({ "id": "framework-app", "_id": "user-value", "mailid": "42" });
        assert_eq!(lookup_param(&params, "id"), Some(&json!("user-value")));
        assert_eq!(lookup_param(&params, "mailid"), Some(&json!("42")));
    }

    #[test]
    fn lookup_falls_back_to_direct_when_no_prefixed_copy() {
        let params = json!({ "id": "framework-app", "mailid": "42" });
        assert_eq!(lookup_param(&params, "id"), Some(&json!("framework-app")));
        assert_eq!(lookup_param(&params, "missing"), None);
    }

    #[test]
    fn unwrap_surfaces_user_values_under_reserved_names() {
        let params = json!({
            "id": "framework-app",
            "_id": "user-value",
            "route": "/mail",
            "_eventId": "evt-7",
            "mailid": "42",
        });
        let unwrapped = unwrap_reserved_keys(&params);
        assert_eq!(
            unwrapped,
            json!({
                "id": "user-value",
                "route": "/mail",
                "eventId": "evt-7",
                "mailid": "42",
            })
        );
    }

    #[test]
    fn app_query_envelope_preserves_reserved_and_underscore_names_independently() {
        let params = json!({
            "id": "order-123", "_id": "literal-id",
            "route": "/details", "_route": "literal-route",
            "eventId": "customer-event", "_eventId": "literal-event",
            "raw": "A&B + 50% / 東京 #1", "tag": "second",
        });
        let payload = json!({
            "_query_params_format": "app",
            "_query_params": params,
            "_query_param_values": {"tag": ["first", "second"]},
        });
        for (name, expected) in params.as_object().unwrap() {
            assert_eq!(
                query_param_result(Some(&payload), name),
                (expected.clone(), true)
            );
        }
        assert_eq!(query_param_result(Some(&payload), ""), (params, true));
        assert_eq!(
            query_param_result(Some(&payload), "missing"),
            (Value::Null, false)
        );
    }

    #[test]
    fn unmarked_notification_payload_still_prefers_legacy_reserved_aliases() {
        let payload = json!({"_query_params": {
            "id": "framework-app", "_id": "order-123", "_eventId": "event-7",
        }});
        assert_eq!(
            query_param_result(Some(&payload), "id"),
            (json!("order-123"), true)
        );
        assert_eq!(
            query_param_result(Some(&payload), "_id"),
            (json!("order-123"), true)
        );
        assert_eq!(
            query_param_result(Some(&payload), ""),
            (
                json!({
                    "id": "order-123", "eventId": "event-7",
                }),
                true
            )
        );
    }

    #[test]
    fn only_the_exact_payload_marker_changes_legacy_behavior() {
        for marker in [Value::Null, json!(true), json!("APP"), json!("unknown")] {
            let payload = json!({
                "_query_params_format": marker,
                "_query_params": { "id": "framework", "_id": "app-data", "_query_params_format": "app" },
            });
            assert_eq!(
                query_param_result(Some(&payload), "id"),
                (json!("app-data"), true)
            );
        }
    }

    #[test]
    fn empty_and_missing_query_data_keep_existing_exists_semantics() {
        assert_eq!(query_param_result(None, ""), (json!({}), true));
        assert_eq!(query_param_result(None, "id"), (Value::Null, false));
        let payload =
            json!({"_query_params_format": "app", "_query_params": {"id": "", "_id": "other"}});
        assert_eq!(query_param_result(Some(&payload), "id"), (json!(""), true));
        assert_eq!(
            query_param_result(Some(&payload), "missing"),
            (Value::Null, false)
        );
    }
}
