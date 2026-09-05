use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::Map};

/// Names reserved by the `/use` route shell. User-supplied params with these
/// names are stored under a `_` prefix in the URL; this node reverses the
/// prefix transparently.
const RESERVED_QUERY_KEYS: &[&str] = &["id", "route", "eventId"];

/// Gets query parameters from the current URL.
///
/// Query parameters are passed via `_query_params` in the workflow payload.
/// For a URL like `/dashboard?tab=settings&page=2`, this would give:
/// `{ "tab": "settings", "page": "2" }`
///
/// Reserved keys (`id`, `route`, `eventId`) are stored under a `_`-prefixed
/// name to avoid colliding with the framework's `/use` shell. This node looks
/// up the prefixed copy first when the requested name is reserved, and when
/// returning all params it surfaces `_id` as `id` (and likewise for the
/// other reserved keys).
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

        let query_params = context
            .get_frontend_query_params()
            .await?
            .unwrap_or(Value::Object(Default::default()));

        let value_pin = context.get_pin_by_name("value").await?;
        let exists_pin = context.get_pin_by_name("exists").await?;

        if param_name.is_empty() {
            let unwrapped = unwrap_reserved_keys(&query_params);
            value_pin.set_value(unwrapped).await;
            exists_pin.set_value(Value::Bool(true)).await;
        } else if let Some(param_value) = lookup_param(&query_params, &param_name) {
            value_pin.set_value(param_value.clone()).await;
            exists_pin.set_value(Value::Bool(true)).await;
        } else {
            value_pin.set_value(Value::Null).await;
            exists_pin.set_value(Value::Bool(false)).await;
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
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
}
