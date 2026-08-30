# Catalog Node Development

## Compilation

Always include the `execute` feature when checking catalog packages:
```bash
cargo check -p flow-like-catalog-<domain> --features execute
```

## on_update

Optional function that runs on every board parse event. Use it to dynamically adjust pins based on user input (e.g., a selection triggers new pins, or regex-parsed values create optional pins). See CallReferenceNode example below.

For abstract memory (thread handles, DB connections), use the context cache with `Any`.

## Tips

- Log warnings and errors for debugging.
- Use `Options` to offer enum dropdowns; set a JSON Schema for struct pins.
- Use `JsonSchema` structs (with their schema in Pin Options) for typed interactions.
- Multiple pins with the same name within one direction let users add more of that type.
- Name the node for FlowScript right after `Node::new`: `node.set_flowscript_name("<namespace>", "<alias>")` (dotted namespace for nesting, camelCase alias) and `node.set_receiver("<pin>")` when it should be callable as a method on that input (`""` keeps it static-only). `lint_catalog::flowscript_names_are_explicit_on_first_party_nodes` fails without them.

## Example: Pure Node

```rust
use flow_like::{
    flow::{
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic},
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[derive(Default)]
pub struct BoolOr {}

impl BoolOr {
    pub fn new() -> Self {
        BoolOr {}
    }
}

#[async_trait]
impl NodeLogic for BoolOr {
    fn get_node(&self) -> Node {
        let mut node = Node::new("bool_or", "Or", "Boolean Or operation", "Utils/Bool");
        node.add_icon("/flow/icons/bool.svg");

        node.add_input_pin(
            "boolean",
            "Boolean",
            "Input Pin for OR Operation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "boolean",
            "Boolean",
            "Input Pin for OR Operation",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "result",
            "Result",
            "OR operation between all boolean inputs",
            VariableType::Boolean,
        );

        return node;
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut output_value = false;
        let boolean_pins = context.get_pins_by_name("boolean").await?;

        for pin in boolean_pins {
            let pin = context.evaluate_pin_ref(pin).await?;
            output_value = output_value || pin;
            if output_value {
                break;
            }
        }

        let result = context.get_pin_by_name("result").await?;
        context.log_message(
            &format!("OR Operation Result: {}", output_value),
            LogLevel::Debug,
        );
        context
            .set_pin_ref_value(&result, json!(output_value))
            .await?;

        return Ok(());
    }
}
```

## Example: Impure Node

```rust
use flow_like::{
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        variable::VariableType,
    },
};
use flow_like_types::async_trait;

#[derive(Default)]
pub struct BranchNode {}

impl BranchNode {
    pub fn new() -> Self {
        BranchNode {}
    }
}

#[async_trait]
impl NodeLogic for BranchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_branch",
            "Branch",
            "Branches the flow based on a condition",
            "Control",
        );
        node.add_icon("/flow/icons/split.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "condition",
            "Condition",
            "The condition to evaluate",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(true)));

        node.add_output_pin(
            "true",
            "True",
            "The flow to follow if the condition is true",
            VariableType::Execution,
        );
        node.add_output_pin(
            "false",
            "False",
            "The flow to follow if the condition is false",
            VariableType::Execution,
        );

        return node;
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let condition = context.evaluate_pin::<bool>("condition").await?;

        let true_pin = context.get_pin_by_name("true").await?;
        let false_pin = context.get_pin_by_name("false").await?;

        context.deactivate_exec_pin_ref(&true_pin).await?;
        context.deactivate_exec_pin_ref(&false_pin).await?;

        if condition {
            context.activate_exec_pin_ref(&true_pin).await?;
        } else {
            context.activate_exec_pin_ref(&false_pin).await?;
        }

        return Ok(());
    }
}
```

## Example: Dynamic Pins with on_update

```rust
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use flow_like::{
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
        node::{Node, NodeLogic},
        pin::PinType,
        variable::VariableType,
    },
};
use flow_like_types::{Value, async_trait, json::from_slice};

#[derive(Default)]
pub struct CallReferenceNode {}

impl CallReferenceNode {
    pub fn new() -> Self {
        CallReferenceNode {}
    }
}

#[async_trait]
impl NodeLogic for CallReferenceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_call_reference",
            "Call Reference",
            "References a specific call in the flow",
            "Control/Call",
        );
        node.add_icon("/flow/icons/workflow.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "fn_ref",
            "Function Reference",
            "The function reference to call",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "The flow to follow if the function call is successful",
            VariableType::Execution,
        );

        return node;
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let fn_ref: String = context.evaluate_pin("fn_ref").await?;

        let mut content_pins = HashMap::with_capacity(context.node.pins.len());
        let input_pins = context.node.pins.clone();

        for (_id, pin) in input_pins {
            let value = context.evaluate_pin_ref::<Value>(pin.clone()).await?;
            let name = pin.lock().await.pin.lock().await.name.clone();
            content_pins.insert(name, value);
        }

        let reference_function = context
            .nodes
            .get(&fn_ref)
            .ok_or_else(|| flow_like_types::anyhow!("Function reference not found"))?;

        let node_ref = reference_function.node.clone();
        let pins = reference_function.pins.clone();
        for (_id, pin) in pins {
            let guard = pin.lock().await;
            let (pin_type, data_type, name) = {
                let pin = guard.pin.lock().await;
                (pin.pin_type.clone(), pin.data_type.clone(), pin.name.clone())
            };
            if pin_type == PinType::Input || data_type == VariableType::Execution {
                continue;
            }
            if let Some(value) = content_pins.get(&name) {
                guard.set_value(value.clone()).await;
            }
        }

        let mut sub_context = context.create_sub_context(reference_function).await;
        sub_context.delegated = true;
        let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
        sub_context.end_trace();
        context.push_sub_context(sub_context);

        if run.is_err() {
            let node_name = node_ref.lock().await.friendly_name.clone();
            let error = run.err().unwrap();
            context.log_message(
                &format!("Error: {:?} calling function {}", error, node_name),
                LogLevel::Error,
            );
        }

        context.activate_exec_pin("exec_out").await?;
        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: Arc<Board>) {
        node.error = None;
        let node_ref = match node.get_pin_by_name("fn_ref") {
            Some(pin) => pin.clone(),
            None => {
                node.error = Some("Function reference pin not found".to_string());
                return;
            }
        };

        let reference = match node_ref.default_value {
            Some(value) => value,
            None => {
                node.error = Some("Function reference pin is not connected".to_string());
                return;
            }
        };

        let reference = match from_slice::<String>(&reference) {
            Ok(value) => value,
            Err(err) => {
                node.error = Some(format!("Failed to parse function reference: {}", err));
                return;
            }
        };

        let event = match board.nodes.get(&reference) {
            Some(event) => event.clone(),
            None => {
                node.error = Some(format!("Function reference not found: {}", reference));
                return;
            }
        };

        node.friendly_name = format!("Call {}", event.friendly_name);
        node.description = format!("Calls the function {}", event.friendly_name);
        node.icon = event.icon.clone();

        let mut output_pins = event
            .pins
            .iter()
            .filter(|pin| {
                pin.1.pin_type == PinType::Output && pin.1.data_type != VariableType::Execution
            })
            .map(|pin| {
                let mut pin = pin.1.clone();
                pin.index += 1;
                pin
            })
            .collect::<Vec<_>>();

        output_pins.sort_by(|a, b| a.index.cmp(&b.index));
        let mut relevant_pins = HashSet::with_capacity(output_pins.len());
        for pin in output_pins {
            relevant_pins.insert(pin.name.clone());
            if node.pins.iter().any(|(_, p)| p.name == pin.name) {
                continue;
            }
            let new_pin = node.add_input_pin(
                &pin.name,
                &pin.friendly_name,
                &pin.description,
                pin.data_type,
            );
            new_pin.schema = pin.schema.clone();
            new_pin.options = pin.options.clone();
        }
        node.pins.retain(|_, pin| {
            if pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution {
                relevant_pins.contains(&pin.name) || pin.name == "fn_ref"
            } else {
                true
            }
        });
    }
}
```

## Example: Feature-Gated Node

```rust
#[cfg(feature = "execute")]
use heavy_pdf_crate::PdfParser;

use flow_like::{
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[derive(Default)]
pub struct ParsePdfNode {}

impl ParsePdfNode {
    pub fn new() -> Self {
        ParsePdfNode {}
    }
}

#[async_trait]
impl NodeLogic for ParsePdfNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "parse_pdf",
            "Parse PDF",
            "Extracts text content from a PDF file",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/pdf.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to the PDF file",
            VariableType::Struct,
        );

        node.add_output_pin("exec_out", "Done", "Execution continues", VariableType::Execution);
        node.add_output_pin("text", "Text", "Extracted text content", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let file_path: String = context.evaluate_pin("file_path").await?;
        let parser = PdfParser::new();
        let text = parser.extract_text(&file_path)?;

        context.set_pin_value("text", json!(text)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PDF parsing requires the 'execute' feature"
        ))
    }
}
```
