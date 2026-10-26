use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct PushArrayNode {}

impl PushArrayNode {
    pub fn new() -> Self {
        PushArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for PushArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_push",
            "Push",
            "Push an item into your Array",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "push");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );

        node.add_input_pin("value", "Value", "Value to push", VariableType::Generic);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Adjusted Array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut array_in = match context.evaluate_pin_to_ref("array_in").await? {
            Value::Array(values) => values,
            // Preserve the existing type error for non-array inputs.
            other => flow_like_types::json::from_value::<Vec<Value>>(other)?,
        };
        let value = context.evaluate_pin_to_ref("value").await?;
        array_in.push(value);
        context
            .set_pin_value("array_out", Value::Array(array_in))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["array_in", "array_out", "value"], true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{
                LogLevel,
                byte_store::{INLINE_BYTES_MAX, handle_hash},
                internal_node::InternalNode,
                internal_pin::InternalPin,
            },
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::{json::json, sync::Mutex, sync::RwLock};
    use std::sync::{Arc, Weak};

    async fn context_for(node: Node) -> ExecutionContext {
        let mut pins = AHashMap::new();
        let mut names: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
        for pin in node.pins.values() {
            let pin = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.to_string())
                .or_default()
                .push(pin.clone());
            pins.insert(pin.id().to_string(), pin);
        }
        let node = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(PushArrayNode::new()),
            names,
        ));
        for pin in node.pins.iter() {
            pin.init_node(Arc::downgrade(&node));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let mut context = ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                node.node_id().to_string(),
                node.clone(),
            )])),
            &Weak::new(),
            &state,
            &node,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Info,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        context.resources =
            Arc::new(flow_like::flow::execution::resources::RunResources::default());
        context
    }

    async fn set_input(context: &ExecutionContext, name: &str, value: Value) {
        context
            .get_pin_by_name(name)
            .await
            .unwrap()
            .set_value(value)
            .await;
    }

    async fn assert_inactive(context: &ExecutionContext) {
        assert_eq!(
            context.evaluate_pin_raw("exec_out").await.unwrap(),
            json!(false)
        );
    }

    #[flow_like_types::tokio::test(crate = "flow_like_types::tokio")]
    async fn push_preserves_nested_values_and_input_array() {
        let mut context = context_for(PushArrayNode::new().get_node()).await;
        let input = json!([{"key": [1, true, null]}, "first"]);
        let value = json!({"nested": {"number": 1.5, "text": "second"}});
        set_input(&context, "array_in", input.clone()).await;
        set_input(&context, "value", value.clone()).await;

        PushArrayNode::new().run(&mut context).await.unwrap();

        assert_eq!(
            context.evaluate_pin_raw("array_out").await.unwrap(),
            json!([{"key": [1, true, null]}, "first", value])
        );
        assert_eq!(context.evaluate_pin_raw("array_in").await.unwrap(), input);
        assert_eq!(
            context.evaluate_pin_raw("exec_out").await.unwrap(),
            json!(true)
        );
    }

    #[flow_like_types::tokio::test(crate = "flow_like_types::tokio")]
    async fn invalid_array_keeps_serde_error_and_is_read_before_value() {
        for input in [
            Value::Null,
            json!(false),
            json!(3),
            json!("text"),
            json!({}),
        ] {
            let mut context = context_for(PushArrayNode::new().get_node()).await;
            set_input(&context, "array_in", input.clone()).await;
            set_input(&context, "exec_out", json!(true)).await;
            let expected = context
                .evaluate_pin::<Vec<Value>>("array_in")
                .await
                .unwrap_err();

            let actual = PushArrayNode::new().run(&mut context).await.unwrap_err();

            assert_eq!(actual.to_string(), expected.to_string());
            assert_eq!(context.evaluate_pin_raw("array_in").await.unwrap(), input);
            assert_inactive(&context).await;
        }
    }

    #[flow_like_types::tokio::test(crate = "flow_like_types::tokio")]
    async fn missing_value_leaves_outputs_and_input_unchanged() {
        let mut context = context_for(PushArrayNode::new().get_node()).await;
        set_input(&context, "array_in", json!([1])).await;
        set_input(&context, "array_out", json!(["previous"])).await;
        set_input(&context, "exec_out", json!(true)).await;
        let expected = context.evaluate_pin::<Value>("value").await.unwrap_err();

        let actual = PushArrayNode::new().run(&mut context).await.unwrap_err();

        assert_eq!(actual.to_string(), expected.to_string());
        assert_eq!(
            context.evaluate_pin_raw("array_in").await.unwrap(),
            json!([1])
        );
        assert_eq!(
            context.evaluate_pin_raw("array_out").await.unwrap(),
            json!(["previous"])
        );
        assert_inactive(&context).await;
    }

    #[flow_like_types::tokio::test(crate = "flow_like_types::tokio")]
    async fn push_expands_root_and_nested_byte_handles_like_typed_reads() {
        for root_handle in [false, true] {
            let mut context = context_for(PushArrayNode::new().get_node()).await;
            let handle = context.bytes_value(vec![42_u8; INLINE_BYTES_MAX + 1]).await;
            assert!(handle_hash(&handle).is_some());
            let input = if root_handle {
                handle.clone()
            } else {
                json!([{"bytes": handle}])
            };
            set_input(&context, "array_in", input.clone()).await;
            set_input(&context, "value", json!({"nested": [handle]})).await;
            let mut expected = context
                .evaluate_pin::<Vec<Value>>("array_in")
                .await
                .unwrap();
            expected.push(context.evaluate_pin::<Value>("value").await.unwrap());

            PushArrayNode::new().run(&mut context).await.unwrap();

            assert_eq!(
                context.evaluate_pin_raw("array_out").await.unwrap(),
                Value::Array(expected)
            );
            assert_eq!(context.evaluate_pin_raw("array_in").await.unwrap(), input);
        }
    }

    #[flow_like_types::tokio::test(crate = "flow_like_types::tokio")]
    async fn missing_byte_handles_keep_the_typed_read_error() {
        let mut context = context_for(PushArrayNode::new().get_node()).await;
        let _scope = context.byte_scope();
        let input = json!([{"$bytes": "0".repeat(64), "len": INLINE_BYTES_MAX + 1}]);
        set_input(&context, "array_in", input.clone()).await;
        set_input(&context, "exec_out", json!(true)).await;
        let expected = context
            .evaluate_pin::<Vec<Value>>("array_in")
            .await
            .unwrap_err();

        let actual = PushArrayNode::new().run(&mut context).await.unwrap_err();

        assert_eq!(actual.to_string(), expected.to_string());
        assert_eq!(context.evaluate_pin_raw("array_in").await.unwrap(), input);
        assert_inactive(&context).await;
    }

    #[flow_like_types::tokio::test(crate = "flow_like_types::tokio")]
    async fn geometry_output_is_validated_before_activation() {
        use flow_like_types::geometry::{GeometryKind, marker};
        let point = json!({"type": "Point", "coordinates": [13.405, 52.52]});
        for value in [
            point.clone(),
            json!({"type": "MultiPoint", "coordinates": []}),
        ] {
            let mut node = PushArrayNode::new().get_node();
            let output = node.get_pin_mut_by_name("array_out").unwrap();
            output.data_type = VariableType::Geometry;
            output.schema = Some(marker(GeometryKind::Point).into());
            let mut context = context_for(node).await;
            set_input(&context, "array_in", Value::Array(Vec::new())).await;
            set_input(&context, "value", value.clone()).await;
            set_input(&context, "array_out", json!([point])).await;
            set_input(&context, "exec_out", json!(true)).await;

            let result = PushArrayNode::new().run(&mut context).await;

            if value == point {
                result.unwrap();
                assert_eq!(
                    context.evaluate_pin_raw("exec_out").await.unwrap(),
                    json!(true)
                );
            } else {
                assert!(result.is_err());
                assert_inactive(&context).await;
            }
            assert_eq!(
                context.evaluate_pin_raw("array_out").await.unwrap(),
                json!([point])
            );
        }
    }
}
