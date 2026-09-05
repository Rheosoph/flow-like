//! Pass a host-owned listener and connection between nodes in one run.

use flow_like_wasm_sdk::*;

fn socket_node(name: &str, label: &str, description: &str) -> NodeDefinition {
    let mut node = NodeDefinition::new(name, label, description, "Custom/WASM/WebSocket");
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
    node.add_permission(NodePermission::NetworkWebsocket);
    node
}

#[register_node]
#[derive(Default)]
pub struct StartServerNode;

impl WasmNode for StartServerNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = socket_node(
            "ws_start_server",
            "Start WebSocket Server",
            "Starts a listener that stays open until closed or this run ends",
        );
        node.add_input_pin(
            "bind_address",
            "Bind Address",
            "IP address and port; port 0 lets the host choose an available port",
            VariableType::String,
        )
        .set_default_value(json!("127.0.0.1:8080"));
        node.add_output_pin(
            "listener",
            "Listener",
            "Opaque listener handle for this package and run",
            VariableType::String,
        );
        node.add_output_pin(
            "address",
            "Address",
            "Bound IP address and port",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let address = ctx
            .get_string("bind_address")
            .unwrap_or_else(|| "127.0.0.1:8080".to_string());
        let Some(listener) = ctx.ws_listen(&address) else {
            return ctx
                .fail("Cannot start listener: check the address, port, and network permission");
        };
        let Some(bound_address) = ctx.ws_local_address(&listener) else {
            ctx.ws_close(&listener);
            return ctx.fail("Listener address is unavailable");
        };
        ctx.set_output("listener", listener);
        ctx.set_output("address", bound_address);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct AcceptConnectionNode;

impl WasmNode for AcceptConnectionNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = socket_node(
            "ws_accept_connection",
            "Accept WebSocket Connection",
            "Waits for a client on a listener created earlier in this run",
        );
        node.add_input_pin(
            "listener",
            "Listener",
            "Listener from Start WebSocket Server in this package and run",
            VariableType::String,
        );
        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait for a client, within the node execution budget",
            VariableType::Integer,
        )
        .set_default_value(json!(10_000));
        node.add_output_pin(
            "connection",
            "Connection",
            "Accepted connection handle for later nodes in this package and run",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let Some(listener) = ctx.get_string("listener") else {
            return ctx.fail("Connect the listener output from Start WebSocket Server");
        };
        let timeout = ctx.get_i64("timeout_ms").unwrap_or(10_000);
        let Ok(timeout) = u32::try_from(timeout) else {
            return ctx.fail("Timeout must be between 0 and 4294967295 milliseconds");
        };
        let Some(connection) = ctx.ws_accept(&listener, timeout) else {
            return ctx
                .fail("No client accepted before the timeout, or the listener is unavailable");
        };
        ctx.set_output("connection", connection);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct SendTextNode;

impl WasmNode for SendTextNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = socket_node(
            "ws_send_text",
            "Send WebSocket Text",
            "Sends text through a connection accepted earlier in this run",
        );
        node.add_input_pin(
            "connection",
            "Connection",
            "Connection from Accept WebSocket Connection",
            VariableType::String,
        );
        node.add_input_pin("text", "Text", "Message to send", VariableType::String)
            .set_default_value(json!("Hello from Flow-Like"));
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        let Some(connection) = ctx.get_string("connection") else {
            return ctx.fail("Connect the connection output from Accept WebSocket Connection");
        };
        let text = ctx.get_string("text").unwrap_or_default();
        if !ctx.ws_send_text(&connection, &text) {
            return ctx.fail("Cannot send: the connection is closed, unavailable, or denied");
        }
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct CloseSocketNode;

impl WasmNode for CloseSocketNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = socket_node(
            "ws_close",
            "Close WebSocket",
            "Closes a connection, or a listener and all clients it accepted",
        );
        node.add_input_pin(
            "handle",
            "Handle",
            "Listener or connection from this package and run",
            VariableType::String,
        );
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        let Some(handle) = ctx.get_string("handle") else {
            return ctx.fail("Connect a listener or connection handle");
        };
        if !ctx.ws_close(&handle) {
            return ctx.fail("Socket is already closed, unavailable, or denied");
        }
        ctx.success()
    }
}
