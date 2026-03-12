//! Flow-Like WASM Node Template — Component Model
//!
//! Uses the `flow-like-wasm-sdk` crate. Mirrors the native catalog pattern:
//! `#[register_node]` + `impl WasmNode` + `wasm_main!()`.
//!
//! # Building
//!
//! ```bash
//! cargo build --release    # outputs a WASM component directly
//! ```
//!
//! The compiled component is at:
//! `target/wasm32-wasip2/release/flow_like_wasm_node_template.wasm`

use flow_like_wasm_sdk::*;

// ── Node 1: Repeat Text ────────────────────────────────────────────────

#[register_node]
#[derive(Default)]
pub struct RepeatTextNode;

impl WasmNode for RepeatTextNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "repeat_text",
            "Repeat Text",
            "Repeats input text N times",
            "Custom/WASM",
        );
        node.add_pin(PinDefinition::input("exec", "Exec", "Trigger pin", "Exec"));
        node.add_pin(
            PinDefinition::input("input_text", "Input Text", "Text to repeat", "String")
                .with_default(json!("")),
        );
        node.add_pin(
            PinDefinition::input("multiplier", "Multiplier", "Number of repetitions", "I64")
                .with_default(json!(1)),
        );
        node.add_pin(PinDefinition::output("exec_out", "Done", "Execution continues", "Exec"));
        node.add_pin(PinDefinition::output(
            "output_text",
            "Output Text",
            "Repeated text result",
            "String",
        ));
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("input_text").unwrap_or_default();
        let mult = ctx.get_i64("multiplier").unwrap_or(1);
        let output = text.repeat(mult.max(0) as usize);
        ctx.set_output("output_text", output);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── Node 2: Character Count ────────────────────────────────────────────

#[register_node]
#[derive(Default)]
pub struct CharCountNode;

impl WasmNode for CharCountNode {
    fn get_node(&self) -> NodeDefinition {
        let mut node = NodeDefinition::new(
            "char_count",
            "Character Count",
            "Counts the number of characters in input text",
            "Custom/WASM",
        );
        node.add_pin(PinDefinition::input("exec", "Exec", "Trigger pin", "Exec"));
        node.add_pin(
            PinDefinition::input("input_text", "Input Text", "Text to measure", "String")
                .with_default(json!("")),
        );
        node.add_pin(PinDefinition::output("exec_out", "Done", "Execution continues", "Exec"));
        node.add_pin(PinDefinition::output(
            "char_count",
            "Char Count",
            "Number of characters",
            "I64",
        ));
        node
    }

    fn run(&self, mut ctx: Context) -> ExecutionResult {
        let text = ctx.get_string("input_text").unwrap_or_default();
        ctx.set_output("char_count", text.len() as i64);
        ctx.activate_exec("exec_out");
        ctx.success()
    }
}

// ── WASM entrypoint (auto-discovers all #[register_node] structs) ──────

wasm_main!();

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_node_definition() {
        let node = RepeatTextNode.get_node();
        assert_eq!(node.name, "repeat_text");
        assert_eq!(node.pins.len(), 5);
    }

    #[test]
    fn count_node_definition() {
        let node = CharCountNode.get_node();
        assert_eq!(node.name, "char_count");
        assert_eq!(node.pins.len(), 4);
    }
}