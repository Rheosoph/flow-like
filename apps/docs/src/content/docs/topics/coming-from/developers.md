---
title: For Developers
description: Translate programming concepts into Flow-Like's typed visual workflows
sidebar:
  order: 3
---

Flow-Like is easiest to understand as a typed, event-driven programming
environment whose source is a graph. Nodes perform work, pins define inputs and
outputs, and wires carry execution or data between nodes.

## Start with the object model

| Flow-Like concept | Closest programming concept |
| --- | --- |
| **App** | Project boundary containing executable logic, interfaces, data, and delivery settings |
| **Flow** | Executable graph; stored internally as a board |
| **Event node** | Entry function inside a Flow |
| **App Event** | Trigger configuration that targets an event node |
| **Node** | Typed operation or function call |
| **Pins and wires** | Function arguments, return values, and control flow |
| **Function** | Reusable, typed function defined within a Flow |
| **Layer** | Nested or collapsed graph used for abstraction |
| **Variable** | Board-level in-memory state |
| **Page or Widget** | User-interface surface backed by Flow data and actions |

These are working analogies, not serialization guarantees. For example, an App
Event is configured outside the graph even though it points to an event node
inside the Flow.

## From function composition to a graph

Traditional code often composes calls directly:

```javascript
const raw = await loadFile(path);
const result = normalize(raw);
await saveOutput(result, outputPath);
```

In Flow-Like, add nodes for the same operations, connect `loadFile` data to
`normalize`, connect the normalized value to `saveOutput`, and connect the
execution pins in the required order.

There are two distinct connection types:

| Connection | Meaning |
| --- | --- |
| **Execution wire** | Determines when a standard node runs |
| **Data wire** | Supplies a typed value to an input pin |

[Standard nodes](/studio/nodes/) run when execution reaches them. Pure nodes
have data pins but no execution pins and evaluate when a downstream node needs
their output. Event nodes are graph entry points.

Flow-Like does not infer that every disconnected branch should run in parallel.
Use [Sequence](/nodes/control/control-sequence/) for ordered fan-out and
[Parallel Execution](/nodes/control/control-par-execution/) or
[Parallel For Each](/nodes/control/control-par-for-each/) when concurrency is
intentional.

## Types, structs, and collections

Pins enforce data types when you connect them. Generic pins can resolve to a
concrete type after a compatible connection is made, and complex values can
also carry a schema.

For example, this interface:

```typescript
interface Customer {
  id: string;
  name: string;
  email: string;
  orders: Order[];
}
```

maps to a struct schema with string fields and a typed array field. Use
[Make Struct](/nodes/structs/struct-make/), [Get Field](/nodes/structs/fields/struct-get/),
and [Set Field](/nodes/structs/fields/struct-set/) to construct and transform
that value.

| Code-level value | Flow-Like representation |
| --- | --- |
| Scalar | String, Integer, Float, Boolean, Date, or another pin type |
| Array | Typed Array value |
| Set or map | Typed Set or Map value |
| Object or record | Struct, optionally constrained by a schema |
| File path | Path value rather than an arbitrary string |
| Runtime handle | Typed reference passed between compatible nodes |

Browse the [generated node catalog](/nodes/overview/) for the current pin and
schema contract of every node.

## Variables and durable state

The Variables panel defines typed state shared by the Flow's graph. Read and
write it with [Get Variable](/nodes/variable/variable-get/) and
[Set Variable](/nodes/variable/variable-set/).

Code such as:

```python
counter += 1
results.append(item)
```

usually becomes a variable read, a typed math or array operation, and a
variable write. Variables are useful for state needed during execution; they
should not be treated as a general durable database.

Choose storage by lifecycle:

| Need | Use |
| --- | --- |
| Temporary execution state | Flow variable |
| Per-device configuration or a secret | [Runtime-configured variable](/apps/runtime-variables/) |
| Files owned by the App | [App Storage](/apps/storage/) |
| Queryable or persistent records | Database nodes and [Data Studio](/apps/data-studio/) |
| Chat conversation context | Chat Event history and session values |

For credentials, mark a variable **Secret** and **Runtime Configured**, then set
its value on the machine that will execute the Flow. Do not place credentials
in ordinary node defaults.

## Control flow

| Programming construct | Current Flow-Like node or pattern |
| --- | --- |
| `if` / `else` | [Branch](/nodes/control/control-branch/) |
| `for item in items` | [For Each](/nodes/control/control-for-each/) |
| Loop with early exit | [For Each (Break)](/nodes/control/control-for-each-with-break/) |
| `while` | [While Loop](/nodes/control/control-while-loop/) |
| Ordered fan-out | [Sequence](/nodes/control/control-sequence/) |
| Parallel work | [Parallel Execution](/nodes/control/control-par-execution/) or [Parallel For Each](/nodes/control/control-par-for-each/) |
| Wait for parallel branches | [Gather](/nodes/control/parallel/control-gather/) |
| Bounded operation | [Timeout](/nodes/control/control-timeout/) |
| Run only once | [Do Once](/nodes/control/flow/control-do-once/) |
| Alternate between two paths | [Flip Flop](/nodes/control/flow/control-flip-flop/) |

Do not translate language-level `try`/`catch` mechanically. Some catalogs, such
as desktop automation, provide explicit recovery nodes; other operations expose
status or result pins that you should validate and branch on. Design the failure
path from the contract of the specific node.

## Functions, layers, and App boundaries

Use the smallest boundary that expresses the intent:

- Define a Flow function and invoke it with
  [Call Function](/nodes/control/functions/control-call-function/) for reusable
  typed logic within the same Flow.
- Collapse a section into a [Layer](/studio/layers/) when it should read as one
  higher-level operation.
- Use an [App Event](/apps/events/) when a user, schedule, API, chat surface, or
  another supported sink must enter the Flow.
- Use [Pages](/apps/pages/) and [Widgets](/apps/widgets/) when the automation
  needs a purpose-built interface.

An event node is analogous to an entry function, but it does not become
externally callable until an App Event is configured to target it.

## I/O and integrations

| Task | Current catalog area |
| --- | --- |
| Build and send an HTTP request | [Web/API nodes](/nodes/web/api/) |
| Read or write file content | [Data/Files nodes](/nodes/data/files/) |
| Send or receive email | [Email nodes](/nodes/email/) |
| Query registered data with SQL | [DataFusion nodes](/nodes/data/datafusion/) |
| Work with structured JSON | [JSON nodes](/nodes/utils/json/) and Struct nodes |
| Record diagnostic output | [Logging nodes](/nodes/logging/) |

Prefer a dedicated integration node when its contract matches the task. Use the
HTTP nodes for an API that does not have a suitable catalog integration.

## Debugging and change control

The Studio keeps run history and node logs. Open a previous run to inspect its
timing and logs or rerun it with the same payload. See
[Logging and tracing](/studio/logging/).

Saved Flow versions are explicit snapshots rather than an automatic commit for
every edit. Production Events can target a saved version instead of the mutable
latest Flow; see [Versioning](/studio/versioning/).

When validating a migration:

1. Run representative success, empty-input, invalid-input, and failure cases.
2. Inspect the data contract at every external boundary.
3. Confirm that local-only nodes run on a compatible machine.
4. Configure runtime variables separately for each execution environment.
5. Pin production Events only after the target Flow version is verified.

## Extend or call Flow-Like from code

If the catalog does not contain the operation you need, custom
[WASM nodes](/dev/wasm-nodes/overview/) provide a documented extension model
with multiple supported source languages. Review the sandbox and manifest
requirements before granting filesystem, network, or other capabilities.

If orchestration should remain in an application, the official
[Node.js and Python SDKs](/dev/sdks/overview/) can trigger workflows, monitor
executions, work with files and databases, and access supported AI endpoints.

## A practical migration sequence

1. Define the input and output types before recreating implementation details.
2. Create one Flow and one event node for a representative entry point.
3. Replace each source operation with a catalog node or a small typed layer.
4. Add explicit Branch, loop, Sequence, or Parallel nodes where ordering matters.
5. Move secrets and environment-specific values to Runtime Variables.
6. Configure the App Event that will invoke the entry node.
7. Exercise the Flow locally, inspect its run history, then choose its execution mode.
8. Extract stable repeated sections into functions or layers.

## Next steps

- [Studio overview](/studio/overview/)
- [Nodes and execution behavior](/studio/nodes/)
- [Typed connections](/studio/connecting/)
- [Variables](/studio/variables/)
- [Events](/apps/events/)
- [Local-only execution](/studio/local-execution/)
