---
applyTo: "packages/catalog/**/*.rs"
---

VERY IMPORTANT: when checking nodes with cargo check probably you need the execute feature!

# Node Creation Guidelines

Apply the [general coding guidelines](./general-coding.instructions.md) to all code.

This document describes the creation process of nodes for Flow-Like. Nodes generally have the run functionality and Pins which hold
data.

## Feature Gating for Heavy Dependencies

**IMPORTANT:** Nodes that use heavy dependencies (PDF parsing, ML inference, email clients, etc.) MUST gate their execution logic behind the `execute` feature flag. This allows API services to compile without these dependencies when they only need to serve node metadata.

### When to use feature gating:
- Dependencies that add significant compile time or binary size
- External service clients (email, databases, HTTP scrapers)
- ML/AI libraries (ONNX, linfa, fastembed)
- Document processing (PDF, Excel, Word parsers)
- Image processing beyond basic operations

### How to gate a node:

1. **In `Cargo.toml`** - Make heavy dependencies optional:
```toml
[features]
execute = ["dep:heavy-crate", "dep:another-heavy-crate"]

[dependencies]
heavy-crate = { workspace = true, optional = true }
```

2. **In the source file** - Gate imports and run() method:
```rust
// Gate heavy imports
#[cfg(feature = "execute")]
use heavy_crate::SomeType;

#[async_trait]
impl NodeLogic for MyNode {
    fn get_node(&self) -> Node {
        // get_node() is NEVER gated - it provides metadata
        let mut node = Node::new(...);
        // ...
        node
    }

    // Gate the run() implementation
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Full implementation using heavy dependencies
        Ok(())
    }

    // Provide fallback when execute feature is disabled
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}
```

3. **Gate helper functions** that use heavy dependencies:
```rust
#[cfg(feature = "execute")]
fn helper_using_heavy_dep() -> SomeType {
    // ...
}
```

### What NOT to gate:
- `get_node()` - Always available for metadata
- `on_update()` - Always available for board parsing
- Basic standard library operations
- Lightweight dependencies (serde, json, etc.)

## Node Creation
Nodes can be Pure and Impure. Pure nodes are those that do not have any side effects and always return the same output for the same input.
Impure nodes are those that may have side effects or return different outputs for the same input.

You will always need Execution Pins on Impure Nodes. No Execution Pins on Pure ones.
Nodes have a `get_node` function that is called from the Node Catalog. It constructs the structure of a node. The `run` function is called when the node is executed.
Here you typically first deactivate outgoing execution pins (in case of failure, this stops the graph execution), next you will execute the logic of the node and finally you will activate the outgoing execution pins and write the data into the pins. Pins hold serialized Serde Values.

Optionally you can add an on_update function that runs on every board parse event and allows to dynamically adjust the pins (for example if a specific selection triggers new pins options, or if you can regex parse a node and extract optional pins that are necessary).

If you need more abstract memory, like a thread-handle or database connections you can use the contexts cache with Any.

## Documentation
- Add a nice node and pin description, so the user understands what the node does.
- Add scores to the node rating: privacy, security, performance, governance, reliability, cost. 0 - 10 (bad - good)

## CRITICAL: Input and Output Pins Must Have Different Names
When a value passes through a node (input → output), the input pin and output pin MUST have different `name` values (first argument). The friendly name (second argument) CAN be the same. Pin names are used by `context.evaluate_pin()` / `context.get_pin_by_name()` and `context.set_pin_value()` to identify which pin to read/write — if an input and output share the same name, get/set operations will collide.

```rust
// WRONG — both share the name "log"
node.add_input_pin("log", "Log", "Log message", VariableType::String);
node.add_output_pin("log", "Log", "Log message", VariableType::String);

// CORRECT — different names, friendly names can match
node.add_input_pin("input_log", "Log", "Log message input", VariableType::String);
node.add_output_pin("output_log", "Log", "Log message output", VariableType::String);
```

Common prefixing conventions: `input_` / `output_`, or semantically distinct names like `source_text` / `result_text`.

## Tipps and Tricks
- Log out warnings, errors etc.
- Multiple Pins with the same name are allowed **within the same direction** (all inputs or all outputs) — they will offer the user to add more pins of this same type to the node.
- Set the Options to offer the user enum drop downs, set a schema for struct pins, it is super helpful.
- If you can, set default values.
- You can abstract inputs using JsonSchemar Structs (and use their schema in the Pin Options) to created typed interactions.
- Success and Error pins should be offered when we leave the workflow system (e.g API calls, Database Connections, etc.). The workflow offers an explicit way to handle errors and success cases out of the box, which can be used in the other cases. IN MOST CASES YOU DO NOT NEED AN ERROR PIN WITH ERROR MESSAGE; JUST RETURN THE ERR()!

## Types to Use
- for Images use: NodeImage
- for Files use: FlowPath

## Updating existing nodes interfaces
If you update the input / output pins of an existing node, dont forget to push the version of the node too!

## Example Nodes
<Example Pure Node>
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
</Example Pure Node>

<Example Simple Impure Node>
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
            context.deactivate_exec_pin_ref(&false_pin).await?;

            return Ok(());
        }

        context.deactivate_exec_pin_ref(&true_pin).await?;
        context.activate_exec_pin_ref(&false_pin).await?;

        return Ok(());
    }
}
</Example Simple Impure Node>



<Example Node with on_update Function>
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
                (
                    pin.pin_type.clone(),
                    pin.data_type.clone(),
                    pin.name.clone(),
                )
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
</Example Node with on_update Function>

<Example Feature-Gated Node (Heavy Dependencies)>
// For nodes that use heavy dependencies like PDF parsing, ML inference, etc.
// The run() method must be gated behind the execute feature

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
    // get_node() is NEVER gated - always provides metadata
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

    // Gated run() - only compiles when execute feature is enabled
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let file_path: String = context.evaluate_pin("file_path").await?;

        // Use heavy dependency
        let parser = PdfParser::new();
        let text = parser.extract_text(&file_path)?;

        context.set_pin_value("text", json!(text)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    // Fallback run() - compiles when execute feature is disabled
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PDF parsing requires the 'execute' feature"
        ))
    }
}
</Example Feature-Gated Node (Heavy Dependencies)>