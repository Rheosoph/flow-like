//! Keep a custom Rust object in package memory and pass its handle between nodes.

use flow_like_wasm_sdk::*;

// This object needs no Serialize, Deserialize, Send, or Sync implementation.
// Only its handle travels through pins; the buffer stays in this Wasm instance.
struct TextBuffer {
    chunks: Vec<String>,
    byte_len: usize,
}

fn buffer_node(name: &str, label: &str, description: &str) -> NodeDefinition {
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

fn add_handle_input(node: &mut NodeDefinition) {
    node.add_input_pin(
        "handle",
        "Buffer",
        "Buffer handle from Create Text Buffer in this package and run",
        VariableType::String,
    );
}

#[register_node]
#[derive(Default)]
pub struct CreateBufferNode;

impl WasmNode for CreateBufferNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = buffer_node(
            "object_create_buffer",
            "Create Text Buffer",
            "Stores a custom Rust buffer for later nodes in this run",
        );
        node.add_input_pin(
            "initial_text",
            "Initial Text",
            "Optional first chunk of text",
            VariableType::String,
        )
        .set_default_value(json!(""));
        node.add_output_pin(
            "handle",
            "Buffer",
            "Opaque handle to the buffer in package memory",
            VariableType::String,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("initial_text").unwrap_or_default();
        let buffer = TextBuffer {
            byte_len: text.len(),
            chunks: vec![text],
        };
        let handle = match resources::insert(buffer) {
            Ok(handle) => handle,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("handle", handle);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct AppendBufferNode;

impl WasmNode for AppendBufferNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = buffer_node(
            "object_append_buffer",
            "Append Text Buffer",
            "Adds a chunk to a buffer created earlier in this run",
        );
        add_handle_input(&mut node);
        node.add_input_pin("text", "Text", "Chunk to append", VariableType::String)
            .set_default_value(json!(""));
        node.add_output_pin(
            "byte_len",
            "Byte Length",
            "Total UTF-8 bytes in the buffer",
            VariableType::Integer,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let Some(handle) = ctx.get_string("handle") else {
            return ctx.fail("Connect the buffer output from Create Text Buffer");
        };
        let text = ctx.get_string("text").unwrap_or_default();
        let byte_len = match resources::with_mut::<TextBuffer, _>(&handle, |buffer| {
            buffer.byte_len += text.len();
            buffer.chunks.push(text);
            buffer.byte_len
        }) {
            Ok(byte_len) => byte_len,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("byte_len", byte_len as i64);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct ReadBufferNode;

impl WasmNode for ReadBufferNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = buffer_node(
            "object_read_buffer",
            "Read Text Buffer",
            "Reads text from a buffer created earlier in this run",
        );
        add_handle_input(&mut node);
        node.add_output_pin(
            "text",
            "Text",
            "All chunks joined in insertion order",
            VariableType::String,
        );
        node.add_output_pin(
            "byte_len",
            "Byte Length",
            "Total UTF-8 bytes in the buffer",
            VariableType::Integer,
        );
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let Some(handle) = ctx.get_string("handle") else {
            return ctx.fail("Connect the buffer output from Create Text Buffer");
        };
        let (text, byte_len) = match resources::with::<TextBuffer, _>(&handle, |buffer| {
            (buffer.chunks.concat(), buffer.byte_len)
        }) {
            Ok(result) => result,
            Err(error) => return ctx.fail(error.to_string()),
        };
        ctx.set_output("text", text);
        ctx.set_output("byte_len", byte_len as i64);
        ctx.success()
    }
}

#[register_node]
#[derive(Default)]
pub struct CloseBufferNode;

impl WasmNode for CloseBufferNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = buffer_node(
            "object_close_buffer",
            "Close Text Buffer",
            "Drops the buffer and invalidates its handle before this run ends",
        );
        add_handle_input(&mut node);
        node
    }

    fn run(&self, ctx: Context) -> ExecutionResult {
        let Some(handle) = ctx.get_string("handle") else {
            return ctx.fail("Connect the buffer output from Create Text Buffer");
        };
        if let Err(error) = resources::close::<TextBuffer>(&handle) {
            return ctx.fail(error.to_string());
        }
        ctx.success()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(name: &str, inputs: serde_json::Value) -> ExecutionResult {
        let entry = inventory::iter::<WasmNodeEntry>
            .into_iter()
            .find(|entry| (entry.get_node)().name == name)
            .expect("buffer node must be registered in the package");
        let ctx = Context::from_input(ExecutionInput {
            inputs: inputs.as_object().unwrap().clone(),
            node_id: format!("{name}-instance"),
            node_name: name.to_string(),
            run_id: "buffer-example-run".to_string(),
            app_id: "example-app".to_string(),
            board_id: "example-board".to_string(),
            user_id: "example-user".to_string(),
            stream_state: false,
            log_level: 0,
        });
        (entry.run)(ctx)
    }

    #[test]
    fn registered_nodes_share_a_custom_object_and_reject_a_closed_handle() {
        let created = dispatch("object_create_buffer", json!({"initial_text": "Hello"}));
        assert!(created.error.is_none(), "{:?}", created.error);
        let handle = created.outputs["handle"].as_str().unwrap();

        let appended = dispatch(
            "object_append_buffer",
            json!({"handle": handle, "text": " 🌍"}),
        );
        assert!(appended.error.is_none(), "{:?}", appended.error);
        assert_eq!(appended.outputs["byte_len"], json!(10));
        assert_eq!(appended.activate_exec, ["exec_out"]);

        let read = dispatch("object_read_buffer", json!({"handle": handle}));
        assert!(read.error.is_none(), "{:?}", read.error);
        assert_eq!(read.outputs["text"], json!("Hello 🌍"));
        assert_eq!(read.outputs["byte_len"], json!(10));

        let closed = dispatch("object_close_buffer", json!({"handle": handle}));
        assert!(closed.error.is_none(), "{:?}", closed.error);
        let stale = dispatch("object_read_buffer", json!({"handle": handle}));
        assert!(stale.error.is_some());
        assert!(stale.activate_exec.is_empty());
    }
}
