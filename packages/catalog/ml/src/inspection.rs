pub mod auto_training;
pub mod auto_training_legacy;
pub mod auto_training_tables;
pub mod feature_engineering;
pub mod learning;
pub mod compute;
pub mod data;
pub mod image;
pub mod lifecycle;
pub mod metrics;
pub mod models;
pub mod sensors;

use flow_like::flow::{node::Node, pin::PinOptions, variable::VariableType};
use schemars::JsonSchema;

fn operation_node<I: JsonSchema + serde::Serialize, O: JsonSchema + serde::Serialize>(
    id: &str,
    name: &str,
    description: &str,
    function: &str,
    category: &str,
) -> Node {
    let mut node = Node::new(id, name, description, category);
    node.set_flowscript_name("inspection", function);
    node.add_icon("/flow/icons/chart-network.svg");
    node.add_input_pin(
        "exec_in",
        "Input",
        "Run this operation",
        VariableType::Execution,
    );
    node.add_input_pin(
        "request",
        "Configuration",
        "Inputs and configuration",
        VariableType::Struct,
    )
    .set_schema::<I>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Done",
        "Operation completed",
        VariableType::Execution,
    );
    let result = node.add_output_pin("result", "Result", "Operation result", VariableType::Struct);
    let schema = flow_like_types::json::json!(schemars::schema_for!(O));
    let is_array = schema.get("type").and_then(|v| v.as_str()) == Some("array");
    let mut item = if is_array {
        schema
            .get("items")
            .cloned()
            .unwrap_or(flow_like_types::json::json!({}))
    } else {
        schema.clone()
    };
    let data_type = match item.get("type").and_then(|v| v.as_str()) {
        Some("boolean") => VariableType::Boolean,
        Some("integer") => VariableType::Integer,
        Some("number") => VariableType::Float,
        Some("string") => VariableType::String,
        _ => VariableType::Struct,
    };
    result.set_data_type(data_type.clone());
    if is_array {
        result.set_value_type(flow_like::flow::pin::ValueType::Array);
    }
    if data_type == VariableType::Struct {
        if item == flow_like_types::json::json!(true) {
            result.set_open_schema();
        } else {
            if is_array {
                if let Some(defs) = schema.get("$defs") {
                    item["$defs"] = defs.clone();
                }
            }
            result.schema = flow_like_types::json::to_string(&item).ok();
        }
    }
    node
}

macro_rules! operation {
    ($node:ident, $id:literal, $name:literal, $description:literal, $function:literal, $category:literal, $input:ty, $output:ty, $handler:path) => {
        #[flow_like_types::async_trait]
        impl flow_like::flow::node::NodeLogic for $node {
            fn get_node(&self) -> flow_like::flow::node::Node {
                super::operation_node::<$input, $output>(
                    $id,
                    $name,
                    $description,
                    $function,
                    $category,
                )
            }
            async fn run(
                &self,
                context: &mut flow_like::flow::execution::context::ExecutionContext,
            ) -> flow_like_types::Result<()> {
                #[cfg(feature = "execute")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let input: $input = context.evaluate_pin("request").await?;
                    let output: $output =
                        tokio::task::spawn_blocking(move || $handler(input)).await??;
                    context
                        .set_pin_value("result", flow_like_types::json::json!(output))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "execute"))]
                {
                    let _ = context;
                    Err(flow_like_types::anyhow!(
                        "ML execution requires the execute feature"
                    ))
                }
            }
        }
    };
}
pub(crate) use operation;

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::ValueType;
    #[test]
    fn generated_pin_metadata_preserves_array_and_scalar_types() {
        let node = operation_node::<Vec<f64>, Vec<f64>>("test", "Test", "Test", "test", "Test");
        let pin = node.get_pin_by_name("result").unwrap();
        assert_eq!(pin.data_type, VariableType::Float);
        assert_eq!(pin.value_type, ValueType::Array);
        let node = operation_node::<bool, bool>("test", "Test", "Test", "test", "Test");
        assert_eq!(
            node.get_pin_by_name("result").unwrap().data_type,
            VariableType::Boolean
        );
    }
    #[test]
    fn registered_inspection_nodes_have_unique_ids_and_valid_schemas() {
        let mut ids = std::collections::HashSet::new();
        for logic in crate::get_catalog() {
            let node = logic.get_node();
            assert!(
                ids.insert(node.name.clone()),
                "Duplicate catalog node {}",
                node.name
            );
            if !node.category.starts_with("AI/ML/") {
                continue;
            }
            for pin in node.pins.values() {
                if let Some(schema) = &pin.schema {
                    flow_like_types::json::from_str::<flow_like_types::Value>(schema).unwrap();
                }
            }
        }
        for required in [
            "ml_auto_train_tabular",
            "ml_auto_train_forecast",
            "ml_auto_train_vision",
            "ml_auto_train_anomaly",
            "ml_get_auto_training",
            "ml_step_auto_training",
            "ml_cancel_auto_training",
            "ml_resume_auto_training",
            "ml_finalize_auto_training",
            "ml_predict_auto_model",
            "ml_export_auto_model",
            "ml_derive_columns",
            "ml_window_features",
            "ml_join_sources",
            "ml_fit_preprocessing",
            "ml_materialize_dataset",
            "ml_create_learning_project",
            "ml_step_learning_project",
            "ml_observe_learning_sample",
            "ml_review_learning_sample",
            "ml_select_samples_for_review",
            "ml_analyze_model_errors",
            "ml_route_learning_project",
            "ml_promote_learning_model",
            "ml_rollback_learning_model",
        ] {
            assert!(
                ids.contains(required),
                "Missing auto training node {required}"
            );
        }
    }
}
