//! Retain an owned iterator across calls, then take back its remaining items.

use flow_like_wasm_sdk::*;

type ItemCursor = std::vec::IntoIter<String>;

fn cursor_node(name: &str, label: &str, description: &str) -> NodeDefinition {
    let mut node = NodeDefinition::new(name, label, description, "Custom/WASM/Objects");
    node.add_input_pin(
        "exec",
        "Exec",
        "Start this operation",
        VariableType::Execution,
    );
    node.add_output_pin(
        "exec_out",
        "Done",
        "Operation completed",
        VariableType::Execution,
    );
    node
}

fn add_cursor_input(node: &mut NodeDefinition) {
    node.add_input_pin(
        "cursor",
        "Cursor",
        "Cursor from Create Item Cursor in this package and run",
        VariableType::String,
    );
}

#[register_node]
#[derive(Default)]
pub struct CreateCursorNode;

impl WasmNode for CreateCursorNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = cursor_node(
            "object_create_cursor",
            "Create Item Cursor",
            "Stores an owned iterator so later calls can consume one item at a time",
        );
        node.add_input_pin(
            "items",
            "Items",
            "Items to consume in order",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(json!(["first", "second", "third"]));
        node.add_output_pin(
            "cursor",
            "Cursor",
            "Handle to the iterator in package memory",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let items: Vec<String> = match ctx.require_input_as("items") {
            Ok(items) => items,
            Err(error) => return ctx.fail(error),
        };
        let cursor = match resources::insert(items.into_iter()) {
            Ok(cursor) => cursor,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("cursor", cursor);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct NextItemNode;

impl WasmNode for NextItemNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = cursor_node(
            "object_next_item",
            "Next Cursor Item",
            "Consumes the next item and keeps the iterator position for another call",
        );
        add_cursor_input(&mut node);
        node.add_output_pin(
            "has_item",
            "Has Item",
            "An item was available",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "item",
            "Item",
            "Next item, or empty when exhausted",
            VariableType::String,
        );
        node.add_output_pin(
            "remaining",
            "Remaining",
            "Number of unconsumed items",
            VariableType::Integer,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let Some(cursor) = ctx.get_string("cursor") else {
            return ctx.fail("Connect the cursor output from Create Item Cursor");
        };
        let (item, remaining) = match resources::with_mut::<ItemCursor, _>(&cursor, |items| {
            (items.next(), items.len())
        }) {
            Ok(result) => result,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("has_item", item.is_some());
        ctx.set_output("item", item.unwrap_or_default());
        ctx.set_output("remaining", remaining as i64);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct FinishCursorNode;

impl WasmNode for FinishCursorNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = cursor_node(
            "object_finish_cursor",
            "Finish Item Cursor",
            "Takes ownership of the iterator, returns its remaining items, and invalidates the handle",
        );
        add_cursor_input(&mut node);
        node.add_output_pin(
            "remaining_items",
            "Remaining Items",
            "Items not consumed by Next Cursor Item",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let Some(cursor) = ctx.get_string("cursor") else {
            return ctx.fail("Connect the cursor output from Create Item Cursor");
        };
        // Removing the object ends its registry lifetime and lets this node
        // consume it with an API that requires ownership, such as collect().
        let items = match resources::remove::<ItemCursor>(&cursor) {
            Ok(items) => items,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("remaining_items", items.collect::<Vec<_>>());
        ctx.success()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(node: &impl WasmNode, inputs: serde_json::Value) -> ExecutionResult {
        node.run(Context::from_input(ExecutionInput {
            inputs: inputs.as_object().unwrap().clone(),
            node_id: "cursor-node".into(),
            node_name: node.get_node().name,
            run_id: "cursor-example-run".into(),
            app_id: "example-app".into(),
            board_id: "example-board".into(),
            user_id: "example-user".into(),
            stream_state: false,
            log_level: 0,
        }))
    }

    #[test]
    fn cursor_handoff_preserves_position_and_finish_consumes_it() {
        let created = run(&CreateCursorNode, json!({"items": ["", "two", "three"]}));
        assert!(created.error.is_none(), "{:?}", created.error);
        let cursor = created.outputs["cursor"].as_str().unwrap();

        let first = run(&NextItemNode, json!({"cursor": cursor}));
        assert!(first.error.is_none(), "{:?}", first.error);
        assert_eq!(first.outputs["has_item"], json!(true));
        assert_eq!(first.outputs["item"], json!(""));
        assert_eq!(first.outputs["remaining"], json!(2));

        let finished = run(&FinishCursorNode, json!({"cursor": cursor}));
        assert!(finished.error.is_none(), "{:?}", finished.error);
        assert_eq!(finished.outputs["remaining_items"], json!(["two", "three"]));
        assert!(run(&NextItemNode, json!({"cursor": cursor}))
            .error
            .is_some());
    }

    #[test]
    fn exhausted_cursor_can_be_finished_and_wrong_types_are_preserved() {
        let created = run(&CreateCursorNode, json!({"items": []}));
        let cursor = created.outputs["cursor"].as_str().unwrap();
        let next = run(&NextItemNode, json!({"cursor": cursor}));
        assert_eq!(next.outputs["has_item"], json!(false));
        let finished = run(&FinishCursorNode, json!({"cursor": cursor}));
        assert_eq!(finished.outputs["remaining_items"], json!([]));

        let other = resources::insert(String::from("keep me")).unwrap();
        assert!(run(&FinishCursorNode, json!({"cursor": other}))
            .error
            .is_some());
        assert_eq!(resources::remove::<String>(&other).unwrap(), "keep me");
    }
}
