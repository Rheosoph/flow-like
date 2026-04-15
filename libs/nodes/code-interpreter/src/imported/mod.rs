pub mod dify_code;
pub mod n8n_code;

use flow_like::flow::{node::Node, variable::VariableType};
use std::collections::HashSet;

#[cfg(feature = "execute")]
use {
    crate::pyodide::runtime::{ExecutionRequest, PyodideRuntime, RuntimeConfig},
    flow_like::flow::execution::context::ExecutionContext,
    flow_like_types::json::json,
    once_cell::sync::OnceCell,
    std::{sync::Arc, time::Duration},
};

pub(crate) const DYN_IN_PREFIX: &str = "dyn_in_";
pub(crate) const DYN_OUT_PREFIX: &str = "dyn_out_";

const STATIC_PINS: &[&str] = &[
    "exec_in",
    "exec_out",
    "exec_error",
    "code",
    "packages",
    "input_schema",
    "output_schema",
    "stdout",
    "stderr",
    "error_msg",
    "success",
];

pub(crate) fn read_pin_string(node: &Node, name: &str) -> Option<String> {
    let pin = node.get_pin_by_name(name)?;
    let raw = pin.default_value.as_ref()?;
    serde_json::from_slice::<String>(raw).ok()
}

fn parse_variable_type(s: &str) -> VariableType {
    match s.to_lowercase().as_str() {
        "string" | "str" | "text" => VariableType::String,
        "float" | "number" | "double" => VariableType::Float,
        "integer" | "int" => VariableType::Integer,
        "boolean" | "bool" => VariableType::Boolean,
        _ => VariableType::Struct,
    }
}

pub(crate) fn update_dynamic_pins(node: &mut Node) {
    let input_schema = read_pin_string(node, "input_schema").unwrap_or_default();
    let output_schema = read_pin_string(node, "output_schema").unwrap_or_default();

    let mut keep: HashSet<String> = STATIC_PINS.iter().map(|s| (*s).to_string()).collect();

    if let Ok(map) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&input_schema)
    {
        for (key, type_val) in &map {
            let pin_name = format!("{DYN_IN_PREFIX}{key}");
            keep.insert(pin_name.clone());
            let var_type = type_val
                .as_str()
                .map(parse_variable_type)
                .unwrap_or(VariableType::Struct);
            if node.get_pin_by_name(&pin_name).is_none() {
                node.add_input_pin(&pin_name, key, &format!("Input: {key}"), var_type);
            }
        }
    }

    if let Ok(map) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&output_schema)
    {
        for (key, type_val) in &map {
            let pin_name = format!("{DYN_OUT_PREFIX}{key}");
            keep.insert(pin_name.clone());
            let var_type = type_val
                .as_str()
                .map(parse_variable_type)
                .unwrap_or(VariableType::Struct);
            if node.get_pin_by_name(&pin_name).is_none() {
                node.add_output_pin(&pin_name, key, &format!("Output: {key}"), var_type);
            }
        }
    }

    node.pins.retain(|_, p| keep.contains(&p.name));
}

#[cfg(feature = "execute")]
static RUNTIME: OnceCell<Arc<PyodideRuntime>> = OnceCell::new();

#[cfg(feature = "execute")]
pub(crate) async fn execute_imported_code(
    context: &mut ExecutionContext,
    wrap_main: bool,
) -> flow_like_types::Result<()> {
    let code: String = context.evaluate_pin("code").await?;
    let packages: Vec<String> = context.evaluate_pin("packages").await.unwrap_or_default();
    let input_schema: String = context
        .evaluate_pin("input_schema")
        .await
        .unwrap_or_default();
    let output_schema: String = context
        .evaluate_pin("output_schema")
        .await
        .unwrap_or_default();

    let mut inputs_map = serde_json::Map::new();
    if let Ok(schema) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&input_schema)
    {
        for key in schema.keys() {
            let pin_name = format!("{DYN_IN_PREFIX}{key}");
            if let Ok(val) = context.evaluate_pin::<serde_json::Value>(&pin_name).await {
                inputs_map.insert(key.clone(), val);
            }
        }
    }

    let final_code = if wrap_main && code.contains("def main(") {
        format!(
            "{code}\n\n_r = main(**inputs)\nif isinstance(_r, dict):\n    outputs.update(_r)\n"
        )
    } else {
        code
    };

    let request = ExecutionRequest {
        code: final_code,
        inputs: serde_json::Value::Object(inputs_map),
        packages,
        package_allowlist: None,
        network_enabled: false,
        allowed_hosts: vec![],
        workspace: None,
        timeout: Duration::from_secs(30),
        memory_limit: 256 * 1024 * 1024,
    };

    let runtime = RUNTIME
        .get_or_try_init(|| PyodideRuntime::new(RuntimeConfig::default()).map(Arc::new))?;
    let response = runtime.execute(request).await;

    if let serde_json::Value::Object(ref outputs) = response.outputs {
        if let Ok(schema) =
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&output_schema)
        {
            for key in schema.keys() {
                let pin_name = format!("{DYN_OUT_PREFIX}{key}");
                if let Some(val) = outputs.get(key) {
                    context.set_pin_value(&pin_name, val.clone()).await?;
                }
            }
        }
    }

    context
        .set_pin_value("stdout", json!(response.stdout))
        .await?;
    context
        .set_pin_value("stderr", json!(response.stderr))
        .await?;
    context
        .set_pin_value("success", json!(response.success))
        .await?;

    if let Some(ref err) = response.error {
        context.set_pin_value("error_msg", json!(err)).await?;
    }

    if response.success {
        context.activate_exec_pin("exec_out").await?;
    } else {
        context.activate_exec_pin("exec_error").await?;
    }

    Ok(())
}
