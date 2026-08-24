use flow_like::a2ui::A2UIElement;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Queries elements by ID pattern matching.
///
/// Supports multiple match types: starts_with, ends_with, contains, and exact.
#[crate::register_node]
#[derive(Default)]
pub struct QueryElementsById;

impl QueryElementsById {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for QueryElementsById {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_query_elements_by_id",
            "Query Elements by ID",
            "Gets elements whose IDs match a pattern",
            "UI/Elements/Query",
        );
        node.set_flowscript_name("ui", "queryElementsById");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "pattern",
            "Pattern",
            "The pattern to match element IDs against",
            VariableType::String,
        );

        node.add_input_pin(
            "match_type",
            "Match Type",
            "How to match: 'starts_with', 'ends_with', 'contains', or 'exact'",
            VariableType::String,
        );

        node.add_output_pin(
            "elements",
            "Elements",
            "Array of matching elements",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "element_ids",
            "Element IDs",
            "Array of matching element IDs",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of matching elements",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let pattern: String = context.evaluate_pin("pattern").await?;
        let match_type: String = context
            .evaluate_pin::<String>("match_type")
            .await
            .unwrap_or_else(|_| "contains".to_string());

        let elements = context.get_frontend_elements().await?;

        let Some(elements_map) = elements else {
            context.log_message("No elements in payload", LogLevel::Warn);
            context
                .get_pin_by_name("elements")
                .await?
                .set_value(Value::Array(vec![]))
                .await;
            context
                .get_pin_by_name("element_ids")
                .await?
                .set_value(Value::Array(vec![]))
                .await;
            context
                .get_pin_by_name("count")
                .await?
                .set_value(Value::Number(0.into()))
                .await;
            return Ok(());
        };

        let mut matching_elements: Vec<Value> = Vec::new();
        let mut matching_ids: Vec<String> = Vec::new();

        for (id, element) in elements_map {
            let matches = match match_type.to_lowercase().as_str() {
                "starts_with" | "startswith" => id.starts_with(&pattern),
                "ends_with" | "endswith" => id.ends_with(&pattern),
                "contains" => id.contains(&pattern),
                "exact" => id == pattern,
                _ => id.contains(&pattern),
            };

            if matches {
                matching_ids.push(id.clone());
                let mut element_with_id = element.clone();
                if let Some(obj) = element_with_id.as_object_mut() {
                    obj.insert("_id".to_string(), Value::String(id.clone()));
                }
                matching_elements.push(element_with_id);
            }
        }

        let count = matching_elements.len() as i64;

        context.log_message(
            &format!(
                "Found {} elements matching pattern '{}' ({})",
                count, pattern, match_type
            ),
            LogLevel::Debug,
        );

        context
            .get_pin_by_name("elements")
            .await?
            .set_value(Value::Array(matching_elements))
            .await;
        context
            .get_pin_by_name("element_ids")
            .await?
            .set_value(Value::Array(
                matching_ids.into_iter().map(Value::String).collect(),
            ))
            .await;
        context
            .get_pin_by_name("count")
            .await?
            .set_value(Value::Number(count.into()))
            .await;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let pattern_pin = match read_only_node.get_pin_by_name("pattern") {
            Some(pin) => pin,
            None => return,
        };

        let pattern = pattern_pin.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).ok()?;
            parsed.as_str().map(String::from)
        });

        if let Some(p) = pattern {
            node.friendly_name = format!("Query '{}'", p);
        } else {
            node.friendly_name = "Query Elements by ID".to_string();
        }
    }
}
