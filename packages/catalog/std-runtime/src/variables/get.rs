use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::schemas_are_compatible,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::{collections::HashMap, sync::Arc};

#[crate::register_node]
#[derive(Default)]
pub struct GetVariable {}

impl GetVariable {
    pub fn new() -> Self {
        GetVariable {}
    }

    pub fn push_registry(registry: &mut HashMap<&'static str, Arc<dyn NodeLogic>>) {
        let node = GetVariable::new();
        let node = Arc::new(node);
        registry.insert("variable_get", node);
    }
}

#[async_trait]
impl NodeLogic for GetVariable {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "variable_get",
            "Get Variable",
            "Get Variable Value",
            "Variable",
        );
        node.set_flowscript_name("variable", "get");

        node.add_icon("/flow/icons/variable.svg");

        node.add_input_pin(
            "var_ref",
            "Variable Reference",
            "The reference to the variable",
            VariableType::String,
        );

        node.add_output_pin(
            "value_ref",
            "Value",
            "The value of the variable",
            VariableType::Generic,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let var_ref: String = context.evaluate_pin("var_ref").await?;
        let (value, sensitive) = context.get_variable_value_ref(&var_ref).await?;

        let value_pin = context.get_pin_by_name("value_ref").await?;
        let value_cloned = value.lock().await.clone();

        if context.log_level <= LogLevel::Debug {
            if sensitive {
                context.log_message("Accessed variable value (hidden)", LogLevel::Debug);
            } else {
                context.log_message(
                    &format!("Accessed variable value: {:?}", value_cloned),
                    LogLevel::Debug,
                );
            }
        }

        context.override_pin_value_if_active(value_pin.id(), &value_cloned);
        value_pin.set_value(value_cloned).await;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let var_ref = match read_only_node.get_pin_by_name("var_ref") {
            Some(pin) => pin,
            None => {
                node.error = Some("Variable not found!".to_string());
                return;
            }
        };

        let var_ref_value = match var_ref.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).unwrap();
            parsed.as_str().map(String::from)
        }) {
            Some(val) => val,
            None => {
                node.error = Some("Variable reference not found!".to_string());
                return;
            }
        };

        let var_ref_variable = match board.get_any_variable(&var_ref_value) {
            Some(var) => var,
            None => {
                node.error = Some("Variable not found!".to_string());
                return;
            }
        };

        let expected_name = format!("Get {}", var_ref_variable.name);

        // Check if anything changed using read_only_node to avoid borrow issues
        let value_pin = read_only_node.get_pin_by_name("value_ref");
        let type_changed = value_pin.is_some_and(|pin| {
            pin.data_type != var_ref_variable.data_type
                || pin.value_type != var_ref_variable.value_type
                || pin.schema != var_ref_variable.schema
        });
        let name_changed = read_only_node.friendly_name != expected_name;

        if !type_changed && !name_changed {
            return;
        }

        node.friendly_name = expected_name;

        let mut_value = match node.get_pin_mut_by_name("value_ref") {
            Some(val) => val,
            None => {
                node.error = Some("Value pin not found!".to_string());
                return;
            }
        };
        let immutable_value = mut_value.clone();

        mut_value.data_type = var_ref_variable.data_type.clone();
        mut_value.value_type = var_ref_variable.value_type.clone();
        mut_value.schema = var_ref_variable.schema.clone();

        if immutable_value.connected_to.is_empty() {
            return;
        }

        let mut connected = immutable_value.connected_to.clone();

        connected.retain(|conn| {
            board.get_pin_by_id(conn).is_some_and(|pin| {
                // Check type and value_type match
                if pin.data_type != mut_value.data_type || pin.value_type != mut_value.value_type {
                    return false;
                }
                if mut_value.data_type == VariableType::Geometry
                    || pin.data_type == VariableType::Geometry
                {
                    return flow_like::flow::pin::geometry_pins_are_compatible(
                        mut_value,
                        pin,
                        &board.refs,
                    )
                    .unwrap_or(false);
                }
                schemas_are_compatible(mut_value.schema.as_deref(), pin.schema.as_deref())
            })
        });

        mut_value.connected_to = connected;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{Run, internal_node::InternalNode, internal_pin::InternalPin},
            pin::ValueType,
            variable::Variable,
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::{
        Cacheable,
        json::{self, json},
        sync::{Mutex, RwLock},
    };
    use std::sync::Weak;

    async fn context_with_variable(variable: Variable, local: bool) -> ExecutionContext {
        let logic: Arc<dyn NodeLogic> = Arc::new(GetVariable::new());
        let mut node = logic.get_node();
        node.get_pin_mut_by_name("var_ref")
            .unwrap()
            .set_default_value(Some(json!(variable.id)));

        let mut pins = AHashMap::new();
        let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
        for pin in node.pins.values() {
            let internal_pin = Arc::new(InternalPin::new(pin, false));
            name_cache
                .entry(pin.name.clone())
                .or_default()
                .push(internal_pin.clone());
            pins.insert(pin.id.clone(), internal_pin);
        }
        let current = Arc::new(InternalNode::new(node, pins, logic, name_cache));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }

        let nodes = Arc::new(AHashMap::from_iter([(
            current.node_id().to_string(),
            current.clone(),
        )]));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let variables = Arc::new(Mutex::new(AHashMap::new()));
        let variable_scope = Arc::new(Mutex::new(AHashMap::from_iter([(
            variable.id.clone(),
            variable,
        )])));
        let cache = Arc::new(RwLock::new(AHashMap::<String, Arc<dyn Cacheable>>::new()));
        let run: Weak<Mutex<Run>> = Weak::new();
        let mut context = ExecutionContext::new(
            nodes,
            &run,
            &state,
            &current,
            if local { &variables } else { &variable_scope },
            &cache,
            LogLevel::Debug,
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
        if local {
            context.local_variables = Some(variable_scope);
        }
        context
    }

    #[tokio::test]
    async fn sensitive_values_reach_output_without_entering_debug_logs() {
        let marker = "sensitive-value-must-not-enter-logs";
        for (secret, runtime_configured) in [(true, false), (false, true), (true, true)] {
            for local in [false, true] {
                let mut variable = Variable::new("Input", VariableType::String, ValueType::Normal);
                variable.secret = secret;
                variable.runtime_configured = runtime_configured;
                *variable.value.lock().await = json!(marker);
                let mut context = context_with_variable(variable, local).await;

                GetVariable::new().run(&mut context).await.unwrap();

                assert_eq!(
                    context
                        .get_pin_by_name("value_ref")
                        .await
                        .unwrap()
                        .get_raw_value()
                        .await,
                    Some(json!(marker))
                );
                let logs = json::to_string(&context.trace.logs).unwrap();
                assert!(!logs.contains(marker));
                assert!(logs.contains("Accessed variable value (hidden)"));
            }
        }
    }

    #[tokio::test]
    async fn ordinary_values_remain_visible_in_debug_logs() {
        let marker = "ordinary-value-visible-in-debug-logs";
        let variable = Variable::new("Input", VariableType::String, ValueType::Normal);
        *variable.value.lock().await = json!(marker);
        let mut context = context_with_variable(variable, false).await;

        GetVariable::new().run(&mut context).await.unwrap();

        let logs = json::to_string(&context.trace.logs).unwrap();
        assert!(logs.contains(marker));
        assert!(!logs.contains("(hidden)"));
    }

    #[tokio::test]
    async fn function_invocations_keep_local_getter_outputs_isolated() {
        let variable = Variable::new("Summaries", VariableType::String, ValueType::HashMap);
        let variable_id = variable.id.clone();
        let parent = context_with_variable(variable.clone(), false).await;
        let variables = HashMap::from([(variable_id.clone(), variable)]);
        let mut first = parent
            .create_function_context(&parent.node, &variables)
            .await;
        let mut second = parent
            .create_function_context(&parent.node, &variables)
            .await;
        let first_value = json!({ "first.txt": "First summary" });
        let second_value = json!({ "second.txt": "Second summary" });
        first
            .set_variable_value(&variable_id, first_value.clone())
            .await
            .unwrap();
        second
            .set_variable_value(&variable_id, second_value.clone())
            .await
            .unwrap();

        // Both calls share the getter pin; the second write must preserve the first call's value.
        GetVariable::new().run(&mut first).await.unwrap();
        GetVariable::new().run(&mut second).await.unwrap();

        assert_eq!(
            first.evaluate_pin_raw("value_ref").await.unwrap(),
            first_value
        );
        assert_eq!(
            second.evaluate_pin_raw("value_ref").await.unwrap(),
            second_value
        );
    }
}
