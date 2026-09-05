use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// These nodes work on whatever struct is handed to them, so "any object" is the honest shape.
/// `PickStructNode` narrows its pins to the connected schema in `on_update`; `MergeStructNode` has
/// no `on_update` and stays open for its whole lifetime.
fn any_struct_pin(pin: &mut flow_like::flow::pin::Pin) {
    pin.set_open_schema();
}

/// Later values win. Nested objects are merged field by field when deep is set,
/// which is what a defaults-plus-overrides config actually needs.
fn merge_into(base: &mut Value, overlay: &Value, deep: bool) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(existing) if deep && existing.is_object() && value.is_object() => {
                        merge_into(existing, value, deep);
                    }
                    _ => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MergeStructNode {}

impl MergeStructNode {
    pub fn new() -> Self {
        MergeStructNode {}
    }
}

#[async_trait]
impl NodeLogic for MergeStructNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_merge",
            "Merge Structs",
            "Lays structs over each other, later ones winning. Useful for defaults plus overrides",
            "Structs",
        );
        node.set_flowscript_name("struct", "merge");
        node.set_receiver("struct");
        node.add_icon("/flow/icons/struct.svg");
        node.set_scores(pure_scores());

        any_struct_pin(node.add_input_pin("struct", "Struct", "Base struct", VariableType::Struct));
        any_struct_pin(node.add_input_pin(
            "struct",
            "Struct",
            "Laid over the base",
            VariableType::Struct,
        ));
        node.add_input_pin(
            "deep",
            "Deep",
            "Merge nested structs field by field instead of replacing them",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "skip_null",
            "Skip Null",
            "Ignore fields that are null in a later struct",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        any_struct_pin(node.add_output_pin(
            "merged",
            "Merged",
            "The combined struct",
            VariableType::Struct,
        ));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let deep: bool = context.evaluate_pin("deep").await?;
        let skip_null: bool = context.evaluate_pin("skip_null").await?;
        let pins = context.get_pins_by_name("struct").await?;

        let mut merged = Value::Object(Default::default());
        for pin in pins {
            let mut overlay: Value = context.evaluate_pin_ref(pin).await?;
            if skip_null && let Value::Object(map) = &mut overlay {
                map.retain(|_, value| !value.is_null());
            }
            merge_into(&mut merged, &overlay, deep);
        }

        context.set_pin_value("merged", merged).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct PickStructNode {}

impl PickStructNode {
    pub fn new() -> Self {
        PickStructNode {}
    }
}

#[async_trait]
impl NodeLogic for PickStructNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_pick",
            "Pick Fields",
            "Keeps only the listed fields, dropping everything else. Use before logging or sending a struct on",
            "Structs/Fields",
        );
        node.set_flowscript_name("struct", "pick");
        node.set_receiver("struct");
        node.add_icon("/flow/icons/struct.svg");
        node.set_scores(pure_scores());

        any_struct_pin(node.add_input_pin(
            "struct",
            "Struct",
            "Input Struct",
            VariableType::Struct,
        ));
        node.add_input_pin(
            "fields",
            "Fields",
            "Top level field names to keep",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_input_pin(
            "mode",
            "Mode",
            "Keep only these fields, or drop them and keep the rest",
            VariableType::String,
        )
        .set_default_value(Some(json!("Keep")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Keep".to_string(), "Drop".to_string()])
                .build(),
        );

        any_struct_pin(node.add_output_pin(
            "result",
            "Result",
            "The projected struct",
            VariableType::Struct,
        ));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut value: Value = context.evaluate_pin("struct").await?;
        let fields: Vec<String> = context.evaluate_pin("fields").await?;
        let mode: String = context.evaluate_pin("mode").await?;

        let keep = mode != "Drop";
        if let Value::Object(map) = &mut value {
            map.retain(|field, _| fields.iter().any(|name| name == field) == keep);
        }

        context.set_pin_value("result", value).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("struct", board, Some(ValueType::Normal), None);
        let _ = node.match_type("result", board, Some(ValueType::Normal), None);
    }
}
