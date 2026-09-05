//! Store standard-library WASI sockets as ordinary package resources.

use flow_like_wasm_sdk::*;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

const MAX_PENDING_BYTES: usize = 1024 * 1024;
const MAX_WRITE_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct Outbox(VecDeque<u8>);

impl Outbox {
    fn queue(&mut self, text: &str) -> Result<(), String> {
        if text.len() > MAX_PENDING_BYTES.saturating_sub(self.0.len()) {
            return Err(
                "Send queue exceeds 1 MiB; poll pending writes before adding more text".into(),
            );
        }
        self.0.extend(text.bytes());
        Ok(())
    }

    fn flush_once(&mut self, stream: &mut impl Write) -> io::Result<usize> {
        if self.0.is_empty() {
            return Ok(0);
        }
        // One bounded, nonblocking write per call. The unsent suffix stays in
        // this object, including after a partial write or WouldBlock result.
        let (first, _) = self.0.as_slices();
        let bytes = &first[..first.len().min(MAX_WRITE_BYTES)];
        match stream.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "connection accepted no bytes",
                ))
            }
            Ok(written) => {
                self.0.drain(..written);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
        Ok(self.0.len())
    }
}

struct Connection {
    stream: TcpStream,
    outbox: Outbox,
}

fn tcp_node(name: &str, label: &str, description: &str) -> NodeDefinition {
    let mut node = NodeDefinition::new(name, label, description, "Custom/WASM/TCP");
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
    // All nodes using these objects must select the same permission domain.
    node.add_permission(NodePermission::NetworkTcp);
    node
}

fn add_listener_input(node: &mut NodeDefinition) {
    node.add_input_pin(
        "listener",
        "Listener",
        "Listener from Start TCP Listener in this package and run",
        VariableType::String,
    );
}

fn add_connection_input(node: &mut NodeDefinition) {
    node.add_input_pin(
        "connection",
        "Connection",
        "Connection from Accept TCP Connection in this package and run",
        VariableType::String,
    );
}

fn add_progress_outputs(node: &mut NodeDefinition) {
    node.add_output_pin(
        "pending_bytes",
        "Pending Bytes",
        "Bytes still queued in this package",
        VariableType::Integer,
    );
    node.add_output_pin(
        "drained",
        "Drained",
        "All queued bytes have been handed to the socket",
        VariableType::Boolean,
    );
}

#[register_node]
#[derive(Default)]
pub struct StartListenerNode;

impl WasmNode for StartListenerNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = tcp_node(
            "tcp_start_listener",
            "Start TCP Listener",
            "Stores a WASI TCP listener for later nodes in this run",
        );
        node.add_input_pin(
            "bind_address",
            "Bind Address",
            "Numeric IP address and port; port 0 selects an available port",
            VariableType::String,
        )
        .set_default_value(json!("127.0.0.1:8080"));
        node.add_output_pin(
            "listener",
            "Listener",
            "Handle to the listener in package memory",
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
            .unwrap_or_else(|| "127.0.0.1:8080".into());
        // Parsing first avoids DNS lookup. This example requests only TCP.
        let address: SocketAddr =
            match address.parse() {
                Ok(address) => address,
                Err(_) => return ctx.fail(
                    "Use a numeric IP address and port, for example 127.0.0.1:8080 or [::1]:8080",
                ),
            };
        let listener = match TcpListener::bind(address) {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!(
                    "Cannot bind TCP listener: {error}; check the address, port, and TCP permission"
                );
                return ctx.fail(message);
            }
        };
        if let Err(error) = listener.set_nonblocking(true) {
            return ctx.fail(format!("Cannot make TCP listener nonblocking: {error}"));
        }
        let address = match listener.local_addr() {
            Ok(address) => address.to_string(),
            Err(error) => return ctx.fail(format!("Cannot read bound TCP address: {error}")),
        };
        let handle = match resources::insert(listener) {
            Ok(handle) => handle,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("listener", handle);
        ctx.set_output("address", address);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct AcceptConnectionNode;

impl WasmNode for AcceptConnectionNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = tcp_node(
            "tcp_accept_connection",
            "Accept TCP Connection",
            "Checks once for an incoming client without waiting",
        );
        add_listener_input(&mut node);
        node.add_output_pin(
            "ready",
            "Ready",
            "A connection was accepted",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "connection",
            "Connection",
            "Connection handle, or empty when no client is ready",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let Some(listener) = ctx.get_string("listener") else {
            return ctx.fail("Connect the listener output from Start TCP Listener");
        };
        let accepted = match resources::with::<TcpListener, _>(&listener, TcpListener::accept) {
            Ok(result) => result,
            Err(error) => return ctx.fail(error.to_string()),
        };
        let stream = match accepted {
            Ok((stream, _)) => stream,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                ctx.set_output("ready", false);
                ctx.set_output("connection", "");
                return ctx.success();
            }
            Err(error) => return ctx.fail(format!("Cannot accept TCP connection: {error}")),
        };
        if let Err(error) = stream.set_nonblocking(true) {
            return ctx.fail(format!("Cannot make TCP connection nonblocking: {error}"));
        }
        let connection = match resources::insert(Connection {
            stream,
            outbox: Outbox::default(),
        }) {
            Ok(handle) => handle,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("ready", true);
        ctx.set_output("connection", connection);
        ctx.success()
    }
}

fn send(mut ctx: Context, text: Option<String>) -> ExecutionResult {
    let Some(connection) = ctx.get_string("connection") else {
        return ctx.fail("Connect the connection output from Accept TCP Connection");
    };
    let pending = resources::with_mut::<Connection, _>(&connection, |connection| {
        if let Some(text) = text {
            connection.outbox.queue(&text)?;
        }
        connection
            .outbox
            .flush_once(&mut connection.stream)
            .map_err(|error| {
                format!("Cannot send TCP bytes: {error}; close the connection after a socket error")
            })
    });
    let pending = match pending {
        Ok(Ok(pending)) => pending,
        Ok(Err(error)) => return ctx.fail(error),
        Err(error) => return ctx.fail(error.to_string()),
    };
    ctx.set_output("pending_bytes", pending as i64);
    ctx.set_output("drained", pending == 0);
    ctx.success()
}

#[register_node]
#[derive(Default)]
pub struct SendTextNode;

impl WasmNode for SendTextNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = tcp_node(
            "tcp_send_text",
            "Queue TCP Text",
            "Queues UTF-8 bytes and attempts one nonblocking write",
        );
        add_connection_input(&mut node);
        node.add_input_pin(
            "text",
            "Text",
            "Bytes to append to the send queue, up to 1 MiB total pending",
            VariableType::String,
        )
        .set_default_value(json!("Hello from Flow-Like\n"));
        add_progress_outputs(&mut node);
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("text").unwrap_or_default();
        send(ctx, Some(text))
    }
}

#[register_node]
#[derive(Default)]
pub struct PollSendNode;

impl WasmNode for PollSendNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = tcp_node(
            "tcp_poll_send",
            "Poll TCP Send",
            "Attempts one nonblocking write of bytes already queued",
        );
        add_connection_input(&mut node);
        add_progress_outputs(&mut node);
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        send(ctx, None)
    }
}

#[register_node]
#[derive(Default)]
pub struct CloseListenerNode;

impl WasmNode for CloseListenerNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = tcp_node("tcp_close_listener", "Close TCP Listener", "Drops the listener; accepted connections remain open until separately closed or the run ends");
        add_listener_input(&mut node);
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        let Some(listener) = ctx.get_string("listener") else {
            return ctx.fail("Connect the listener output from Start TCP Listener");
        };
        if let Err(error) = resources::close::<TcpListener>(&listener) {
            return ctx.fail(error.to_string());
        }
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct CloseConnectionNode;

impl WasmNode for CloseConnectionNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = tcp_node(
            "tcp_close_connection",
            "Close TCP Connection",
            "Drops the connection and discards any bytes still queued in package memory",
        );
        add_connection_input(&mut node);
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        let Some(connection) = ctx.get_string("connection") else {
            return ctx.fail("Connect the connection output from Accept TCP Connection");
        };
        if let Err(error) = resources::close::<Connection>(&connection) {
            return ctx.fail(error.to_string());
        }
        ctx.success()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Duration;

    struct LimitedWriter {
        limit: usize,
        error: Option<io::ErrorKind>,
        received: Vec<u8>,
    }

    impl Write for LimitedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(kind) = self.error.take() {
                return Err(io::Error::from(kind));
            }
            let count = self.limit.min(bytes.len());
            self.received.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partial_writes_and_backpressure_preserve_queued_bytes() {
        let mut outbox = Outbox::default();
        outbox.queue("hello 🌍").unwrap();
        let mut writer = LimitedWriter {
            limit: 2,
            error: Some(io::ErrorKind::WouldBlock),
            received: vec![],
        };
        assert_eq!(outbox.flush_once(&mut writer).unwrap(), 10);
        assert_eq!(outbox.flush_once(&mut writer).unwrap(), 8);
        outbox.queue("!").unwrap();
        writer.error = Some(io::ErrorKind::Interrupted);
        assert_eq!(outbox.flush_once(&mut writer).unwrap(), 9);
        while outbox.flush_once(&mut writer).unwrap() > 0 {}
        assert_eq!(writer.received, "hello 🌍!".as_bytes());
    }

    #[test]
    fn send_work_and_queue_memory_are_bounded() {
        let mut outbox = Outbox::default();
        outbox.queue(&"x".repeat(MAX_PENDING_BYTES)).unwrap();
        assert!(outbox.queue("!").is_err());
        let mut writer = LimitedWriter {
            limit: usize::MAX,
            error: None,
            received: vec![],
        };
        assert_eq!(
            outbox.flush_once(&mut writer).unwrap(),
            MAX_PENDING_BYTES - MAX_WRITE_BYTES
        );
        assert_eq!(writer.received.len(), MAX_WRITE_BYTES);
        writer.limit = 0;
        assert_eq!(
            outbox.flush_once(&mut writer).unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
        assert_eq!(outbox.0.len(), MAX_PENDING_BYTES - MAX_WRITE_BYTES);
    }

    fn run(node: &impl WasmNode, inputs: serde_json::Value) -> ExecutionResult {
        node.run(Context::from_input(ExecutionInput {
            inputs: inputs.as_object().unwrap().clone(),
            node_id: "tcp-node".into(),
            node_name: node.get_node().name,
            run_id: "tcp-example-run".into(),
            app_id: "example-app".into(),
            board_id: "example-board".into(),
            user_id: "example-user".into(),
            stream_state: false,
            log_level: 0,
        }))
    }

    #[test]
    fn nodes_handoff_sockets_and_close_each_resource_independently() {
        let started = run(&StartListenerNode, json!({"bind_address": "127.0.0.1:0"}));
        assert!(started.error.is_none(), "{:?}", started.error);
        let listener = started.outputs["listener"].as_str().unwrap();
        let waiting = run(&AcceptConnectionNode, json!({"listener": listener}));
        assert!(waiting.error.is_none(), "{:?}", waiting.error);
        assert_eq!(waiting.outputs["ready"], json!(false));
        assert_eq!(waiting.outputs["connection"], json!(""));

        // A typed close must not remove a different resource.
        assert!(run(&CloseConnectionNode, json!({"connection": listener}))
            .error
            .is_some());
        let mut client = TcpStream::connect(started.outputs["address"].as_str().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut accepted = run(&AcceptConnectionNode, json!({"listener": listener}));
        for _ in 0..100 {
            if accepted.outputs["ready"] == json!(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
            accepted = run(&AcceptConnectionNode, json!({"listener": listener}));
        }
        assert!(accepted.error.is_none(), "{:?}", accepted.error);
        assert_eq!(accepted.outputs["ready"], json!(true));
        let connection = accepted.outputs["connection"].as_str().unwrap();
        assert!(run(&CloseListenerNode, json!({"listener": listener}))
            .error
            .is_none());
        assert!(run(&AcceptConnectionNode, json!({"listener": listener}))
            .error
            .is_some());

        let sent = run(
            &SendTextNode,
            json!({"connection": connection, "text": "hello"}),
        );
        assert!(sent.error.is_none(), "{:?}", sent.error);
        assert_eq!(sent.outputs["drained"], json!(true));
        let mut received = [0; 5];
        client.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"hello");
        assert!(run(&CloseConnectionNode, json!({"connection": connection}))
            .error
            .is_none());
        assert_eq!(client.read(&mut received).unwrap(), 0);
        assert!(run(&PollSendNode, json!({"connection": connection}))
            .error
            .is_some());
    }
}
