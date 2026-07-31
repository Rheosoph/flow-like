---
title: Writing Native Nodes
description: Add Rust nodes to Flow-Like's built-in catalog
sidebar:
  order: 20
---

Native nodes are Rust implementations compiled into Flow-Like's catalog. Use
them when contributing a generally useful node to this repository. For private
or independently distributed extensions, use
[WASM nodes](/dev/wasm-nodes/overview/) instead.

## The `NodeLogic` contract

A native node implements `NodeLogic` from
`packages/core/src/flow/node.rs`:

```rust
#[async_trait]
pub trait NodeLogic: Send + Sync {
    fn get_node(&self) -> Node;

    async fn run(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()>;

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}
```

- `get_node` defines stable metadata, pins, defaults, and presentation.
- `run` reads inputs, writes outputs, and activates execution pins.
- `on_update` is optional and updates a placed node when its definition depends
  on board state or pin defaults.

Register implementations with the catalog attribute and make them
constructible with `Default`:

```rust
#[crate::register_node]
#[derive(Default)]
pub struct AddIntegerNode;
```

The node's internal name is its persistent logic identifier. Treat it as an API:
do not rename it after boards have started using the node.

## A pure data node

A pure node has no execution pins and must not produce side effects. The runtime
may evaluate it on demand or cache its result. The integer-add node in
`packages/catalog/std/src/utils/int/add.rs` is a compact example:

```rust
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct AddIntegerNode;

#[async_trait]
impl NodeLogic for AddIntegerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "int_add",
            "+",
            "Adds two integers",
            "Math/Int",
        );
        node.add_icon("/flow/icons/sigma.svg");
        node.add_input_pin(
            "integer1",
            "Integer 1",
            "First integer",
            VariableType::Integer,
        );
        node.add_input_pin(
            "integer2",
            "Integer 2",
            "Second integer",
            VariableType::Integer,
        );
        node.add_output_pin(
            "sum",
            "Sum",
            "Sum of both integers",
            VariableType::Integer,
        );
        node
    }

    async fn run(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let left: i64 = context.evaluate_pin("integer1").await?;
        let right: i64 = context.evaluate_pin("integer2").await?;
        context.set_pin_value("sum", json!(left + right)).await?;
        Ok(())
    }
}
```

The Branch node is **not** pure: its input and outputs include execution pins.

## An execution node

Execution pins control which downstream path runs. A branch reads its condition,
deactivates both outputs, then activates the selected output:

```rust
async fn run(
    &self,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    let condition: bool = context.evaluate_pin("condition").await?;
    let true_pin = context.get_pin_by_name("true").await?;
    let false_pin = context.get_pin_by_name("false").await?;

    context.deactivate_exec_pin_ref(&true_pin).await?;
    context.deactivate_exec_pin_ref(&false_pin).await?;

    if condition {
        context.activate_exec_pin_ref(&true_pin).await?;
    } else {
        context.activate_exec_pin_ref(&false_pin).await?;
    }

    Ok(())
}
```

See `packages/catalog/std/src/control/branch_node.rs` for the complete
definition.

## Reading and writing pins

Pin values cross the runtime boundary as JSON and are deserialized into the type
you request:

```rust
let text: String = context.evaluate_pin("text").await?;
let limit: i64 = context.evaluate_pin("limit").await?;

context
    .set_pin_value("result", flow_like_types::json::json!(text))
    .await?;
```

When repeatedly accessing one pin, resolve it once with
`get_pin_by_name` and use the reference-based context methods.

Pin names are also persistent interface identifiers. Prefer descriptive,
lowercase names and keep them stable. Each node and pin should have a useful
description because the editor surfaces that text directly.

## Schemas and options

Struct pins can carry a JSON Schema generated from a Rust type:

```rust
node.add_input_pin(
    "bit",
    "Model Bit",
    "Model configuration",
    VariableType::Struct,
)
.set_schema::<Bit>();
```

To make the schema a connection constraint rather than editor guidance:

```rust
node.add_input_pin(
    "bit",
    "Model Bit",
    "Model configuration",
    VariableType::Struct,
)
.set_schema::<Bit>()
.set_options(PinOptions::new().set_enforce_schema(true).build());
```

Schemas are stored in the board's schema references rather than copied inline
into every node.

Some nodes deliberately define repeated pins with the same name so the editor
can add more instances. The catalog lint checks enforce the supported shape;
use an existing variadic node as a reference before introducing this pattern.

## Dynamic definitions

Use `on_update(&mut Node, &Board)` when a node's visible pins depend on its
configuration. The Format String node scans placeholders such as `{customer}`
and keeps corresponding generic input pins in sync:

```rust
async fn on_update(&self, node: &mut Node, board: &Board) {
    // Read configuration from node defaults.
    // Add missing pins and remove stale pins.
    // Reconcile connected types against `board`.
}
```

See `packages/catalog/std/src/utils/string/format.rs` for the full,
duplicate-safe implementation.

`on_update` runs frequently while a board is edited. Keep it deterministic,
idempotent, and inexpensive. Preserve existing pin IDs whenever possible so
connections are not needlessly discarded.

## Node scores

`NodeScores` contains `privacy`, `security`, `performance`, `governance`,
`reliability`, and `cost`, each from `0` to `10`. These are impact/risk
indicators: a higher number means greater impact in that category, not “better
quality.” In particular, a higher `performance` score means worse performance.

Only add scores you can justify consistently with comparable catalog nodes.

## Validate a contribution

Run the catalog definition checks after adding or changing native nodes:

```bash
mise run test:catalog:lint
```

Also run a targeted package test or check for the catalog you changed. For the
standard catalog:

```bash
cargo test -p flow-like-catalog-std
```

Before opening a pull request, follow the repository-wide steps in
[Contributing](/dev/contribute/).
