//! Python interpreter node — wraps the Pyodide / CPython-WASI execution engine.
//!
//! # Security model
//! All sandbox constraints are **node-input driven** so LLM agents and human
//! users can tune isolation per invocation:
//!
//! | Pin | Default | Effect |
//! |-----|---------|--------|
//! | `network_enabled` | `false` | WASI sockets are unavailable when off |
//! | `network_allowlist` | `[]` | Best-effort Python-level hostname filter when network is on |
//! | `packages` | `[]` | micropip packages to install before running code |
//! | `package_allowlist` | null | `null` = any pkg; `[]` = none; `["p"]` = listed only |
//! | `timeout_secs` | `30` | Hard epoch-based timeout — kills the WASM instance |
//! | `max_memory_mb` | `256` | Linear memory cap enforced by Wasmtime |

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
#[cfg(feature = "execute")]
use flow_like_types::Value;

#[cfg(feature = "execute")]
use {
    super::runtime::{ExecutionRequest, PyodideRuntime, RuntimeConfig, WorkspaceInfo},
    once_cell::sync::OnceCell,
    std::{sync::Arc, time::Duration},
};

#[cfg(feature = "execute")]
static RUNTIME: OnceCell<Arc<PyodideRuntime>> = OnceCell::new();

// ─────────────────────────────────────────────────────────────────────────────

#[crate::register_node]
#[derive(Default)]
pub struct PythonInterpreterNode {}

#[async_trait]
impl NodeLogic for PythonInterpreterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "python_interpreter",
            "Python Interpreter",
            "Execute Python code inside a secure WASM (Pyodide / CPython-WASI) sandbox. \
             The sandbox has no host filesystem access. \
             Network access is disabled by default.",
            "Code/Python",
        );
        node.set_long_running(true);
        node.add_icon("/flow/icons/code.svg");

        // ── Execution flow ────────────────────────────────────────────────
        node.add_input_pin(
            "exec_in",
            "Execute",
            "Trigger execution",
            VariableType::Execution,
        );

        // ── Python source ─────────────────────────────────────────────────
        node.add_input_pin(
            "code",
            "Code",
            "Python source to execute.\n\
             Available globals:\n\
             • `inputs`    — dict with values from the Inputs pin\n\
             • `outputs`   — write your results here\n\
             • `workspace` — Workspace object (see Workspace pin):\n\
             \t  workspace.list(prefix=\"\")   → [\"path\", ...]\n\
             \t  workspace.get(\"path\")       → bytes | None\n\
             \t  workspace.put(\"path\", data) → None",
            VariableType::String,
        )
        .set_default_value(Some(json!(
            "# Flow-Like Python Interpreter\n\
             # inputs    — dict with values from the Inputs pin\n\
             # outputs   — write your results here\n\
             # workspace — optional workspace API:\n\
             #   workspace.list()         → list of paths\n\
             #   workspace.get(path)      → bytes or None\n\
             #   workspace.put(path, b\"\") → stage for upload\n\
             \n\
             outputs['result'] = inputs.get('value', 'Hello World')\n"
        )));

        // ── Input data ────────────────────────────────────────────────────
        node.add_input_pin(
            "inputs",
            "Inputs",
            "Arbitrary JSON/Struct data exposed as the `inputs` dict inside Python.",
            VariableType::Struct,
        );

        // ── Workspace (FlowPath) ──────────────────────────────────────────
        node.add_input_pin(
            "workspace",
            "Workspace",
            "Optional FlowPath workspace exposed to Python via the `workspace` API object.\n\
             \n\
             Files are fetched **on demand** — the full workspace is never pre-downloaded,\n\
             so large workspaces are fully supported.\n\
             \n\
             Python usage:\n\
             \t workspace.list(prefix=\"\")   → list of relative paths\n\
             \t workspace.get(\"dir/file\")   → bytes or None\n\
             \t workspace.put(\"out\", data)  → staged for upload after execution\n\
             \n\
             All backends are supported (local, S3, Azure, GCS, …).\n\
             Path traversal above the workspace prefix is silently blocked.\n\
             Leave disconnected for no workspace access.",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        // ── Package installation ──────────────────────────────────────────
        node.add_input_pin(
            "packages",
            "Packages",
            "micropip package names to install before execution \
             (e.g. [\"numpy\", \"requests\"]).\n\
             Requires the Python runtime to include micropip.",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "package_allowlist",
            "Package Allowlist",
            "Controls which packages may be installed via micropip.\n\
             • Not connected / null — any package is allowed\n\
             • [] (empty array)     — no packages allowed\n\
             • [\"pkg\", …]         — only listed names are allowed",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        // no default → evaluates to null → unrestricted

        // ── Network ───────────────────────────────────────────────────────
        node.add_input_pin(
            "network_enabled",
            "Network Enabled",
            "Allow the Python sandbox to open network connections.\n\
             DISABLED by default — enable only when required.\n\
             Note: when enabled, all hosts are reachable unless \
             `network_allowlist` is configured.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "network_allowlist",
            "Network Allowlist",
            "Hostname allowlist applied as a best-effort Python socket patch \
             when network is enabled (e.g. [\"pypi.org\", \"files.pythonhosted.org\"]).\n\
             Empty list = all hosts permitted.\n\
             ⚠️  This is a Python-level guard only; true enforcement \
             requires a network proxy.",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        // ── Resource limits ───────────────────────────────────────────────
        node.add_input_pin(
            "timeout_secs",
            "Timeout (s)",
            "Hard execution time limit in seconds.\n\
             The WASM instance is killed via epoch interruption when exceeded.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(30.0)))
        .set_options(PinOptions::new().set_range((1.0, 300.0)).build());

        node.add_input_pin(
            "max_memory_mb",
            "Max Memory (MB)",
            "Maximum linear memory the Python WASM instance may allocate.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(256.0)))
        .set_options(PinOptions::new().set_range((32.0, 1024.0)).build());

        // ── Outputs ───────────────────────────────────────────────────────
        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated when execution completes without an unhandled exception.",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Activated when execution raises an unhandled exception or times out.",
            VariableType::Execution,
        );
        node.add_output_pin(
            "result",
            "Result",
            "Contents of the Python `outputs` dict after execution.",
            VariableType::Struct,
        );
        node.add_output_pin(
            "stdout",
            "Stdout",
            "Captured standard output from the Python code.",
            VariableType::String,
        );
        node.add_output_pin(
            "stderr",
            "Stderr",
            "Captured standard error from the Python code.",
            VariableType::String,
        );
        node.add_output_pin(
            "error_msg",
            "Error Message",
            "Full traceback / error message when execution fails.",
            VariableType::String,
        );
        node.add_output_pin(
            "success",
            "Success",
            "True when the Python code completed without an unhandled exception.",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            self.do_run(context).await
        })
    }
}

// ─── Execution impl (execute feature only) ───────────────────────────────────

#[cfg(feature = "execute")]
impl PythonInterpreterNode {
    async fn do_run(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        // ── Read inputs ───────────────────────────────────────────────────
        let code: String = context.evaluate_pin("code").await?;

        let inputs: Value = context
            .evaluate_pin("inputs")
            .await
            .unwrap_or(Value::Object(Default::default()));

        let packages: Vec<String> = context
            .evaluate_pin("packages")
            .await
            .unwrap_or_default();

        let package_allowlist: Option<Vec<String>> =
            match context.evaluate_pin::<Value>("package_allowlist").await {
                Ok(Value::Array(arr)) => Some(
                    arr.into_iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                ),
                _ => None,
            };

        let network_enabled: bool = context
            .evaluate_pin("network_enabled")
            .await
            .unwrap_or(false);

        let network_allowlist: Vec<String> = context
            .evaluate_pin("network_allowlist")
            .await
            .unwrap_or_default();

        let timeout_secs: f64 =
            context.evaluate_pin("timeout_secs").await.unwrap_or(30.0);

        let max_memory_mb: f64 = context
            .evaluate_pin("max_memory_mb")
            .await
            .unwrap_or(256.0);

        // ── Workspace info ────────────────────────────────────────────────
        // Extract the object store + prefix from the FlowPath pin.
        // The runtime spawns a file server that services workspace.get()
        // requests lazily — nothing is downloaded upfront.
        let workspace = resolve_workspace_info(context).await;

        // ── Build the request ─────────────────────────────────────────────
        let request = ExecutionRequest {
            code,
            inputs,
            packages,
            package_allowlist,
            network_enabled,
            allowed_hosts: network_allowlist,
            workspace,
            timeout: Duration::from_secs_f64(timeout_secs.max(1.0)),
            memory_limit: (max_memory_mb.max(32.0) * 1024.0 * 1024.0) as usize,
        };

        // ── Get (or lazily init) the shared runtime ───────────────────────
        let runtime = RUNTIME
            .get_or_try_init(|| {
                PyodideRuntime::new(RuntimeConfig::default()).map(Arc::new)
            })?;

        context.log_message("Starting Python sandbox execution…", LogLevel::Debug);

        let response = runtime.execute(request).await;

        // ── Write outputs ─────────────────────────────────────────────────
        context.set_pin_value("result", response.outputs).await?;
        context.set_pin_value("stdout", json!(response.stdout)).await?;
        context.set_pin_value("stderr", json!(response.stderr)).await?;
        context.set_pin_value("success", json!(response.success)).await?;

        if let Some(ref err) = response.error {
            context.set_pin_value("error_msg", json!(err)).await?;
            context.log_message(
                &format!("Python execution failed: {err}"),
                LogLevel::Error,
            );
        }

        // ── Activate exec path ────────────────────────────────────────────
        if response.success {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_error").await?;
        }

        Ok(())
    }
}

// ─── Workspace helper ─────────────────────────────────────────────────────────

/// Resolve the `workspace` pin to a [`WorkspaceInfo`] if the pin is connected
/// and the store is reachable.  Returns `None` silently on any error.
#[cfg(feature = "execute")]
async fn resolve_workspace_info(context: &mut ExecutionContext) -> Option<WorkspaceInfo> {
    let raw: Value = context.evaluate_pin("workspace").await.ok()?;
    if !raw.is_object() {
        return None;
    }
    let flow_path: FlowPath = flow_like_types::json::from_value(raw).ok()?;
    let prefix = flow_path.path.clone();

    let store = match flow_path.to_store(context).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!("workspace: could not resolve store: {e}");
            return None;
        }
    };

    Some(WorkspaceInfo {
        store: store.as_generic(),
        prefix,
    })
}
