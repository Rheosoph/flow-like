---
title: For Unreal Engine Developers
description: Apply Unreal Blueprint graph skills to Flow-Like automation
sidebar:
  order: 5
---

Unreal Blueprint experience transfers well to Flow-Like's node canvas: both
distinguish execution from data, enforce pin types, and support pure and impure
nodes. The domain is different. Flow-Like runs event-driven automation, data,
AI, and interface workflows; it is not a game loop, world, or gameplay runtime.

## Translate the graph model

| Blueprint concept | Closest Flow-Like concept |
| --- | --- |
| Unreal project | App |
| Blueprint Event Graph | Flow |
| Blueprint asset | Flow plus its functions, variables, and metadata |
| Node | Node |
| Execution pin and wire | Execution pin and white wire |
| Data pin and wire | Typed data pin and colored dashed wire |
| Impure node | Standard node |
| Pure function | Pure node |
| Variable | Flow variable |
| Function | Flow function invoked by Call Function |
| Collapsed graph or macro-like grouping | Layer |
| Custom Event | Event node targeted by an App Event |
| Branch | Branch |
| For Each Loop | For Each |
| Sequence | Sequence |
| Flip Flop | Flip Flop |
| Do Once | Do Once |
| Struct | Schema-constrained Struct |
| Array | Typed Array |
| Reroute Node | Reroute |

The mapping is conceptual. A Flow is stored internally as a board, but the App
and Studio interfaces present it as a Flow. It does not behave like a spawned
Blueprint instance.

## Execution and data wires

[Execution wires](/studio/connecting/) determine when standard nodes run. Data
wires supply typed values. A pure node has no execution pin and evaluates when
a downstream consumer needs its output.

This is close to Blueprint's pure/impure distinction, with two cautions:

- multiple outgoing wires are not a substitute for the
  [Sequence](/nodes/control/control-sequence/) node when order matters;
- concurrency should be explicit through
  [Parallel Execution](/nodes/control/control-par-execution/) or
  [Parallel For Each](/nodes/control/control-par-for-each/), with
  [Gather](/nodes/control/parallel/control-gather/) when branches must rejoin.

Pins must have compatible types. Generic pins can resolve to a concrete type
after connection, and complex Struct pins may enforce a schema.

## Familiar control nodes

| Blueprint pattern | Current Flow-Like node |
| --- | --- |
| Boolean branch | [Branch](/nodes/control/control-branch/) |
| Iterate an Array | [For Each](/nodes/control/control-for-each/) |
| Iterate with early exit | [For Each (Break)](/nodes/control/control-for-each-with-break/) |
| Condition-controlled loop | [While Loop](/nodes/control/control-while-loop/) |
| Ordered outputs | [Sequence](/nodes/control/control-sequence/) |
| Alternate A and B | [Flip Flop](/nodes/control/flow/control-flip-flop/) |
| Allow one execution until reset | [Do Once](/nodes/control/flow/control-do-once/) |
| Bounded execution | [Timeout](/nodes/control/control-timeout/) |
| Visual rerouting | [Reroute](/nodes/control/reroute/) |

Use these nodes directly rather than recreating Flip Flop, Do Once, or Sequence
with ad hoc variables and wires.

## Functions and layers

A Flow function is the closest match for reusable Blueprint function logic.
Define typed inputs and outputs on the function, then invoke it with
[Call Function](/nodes/control/functions/control-call-function/).

[Layers](/studio/layers/) collapse a group of nodes behind a typed placeholder.
They are useful for readability and prototyping and can be nested. Use a
function when the graph should be called as reusable logic; use a Layer when
the main goal is a named abstraction inside the canvas.

Unlike Blueprint inheritance or components, Layers do not create Actors,
objects, or a world hierarchy.

## Structs, arrays, and fields

Define a Struct schema for records that need stable fields. The current catalog
includes:

| Need | Node |
| --- | --- |
| Build a Struct | [Make Struct](/nodes/structs/struct-make/) |
| Build from a schema | [Make Struct (Schema)](/nodes/structs/struct-make-from-schema/) |
| Read one field | [Get Field](/nodes/structs/fields/struct-get/) |
| Update one field | [Set Field](/nodes/structs/fields/struct-set/) |
| Expose all fields as pins | [Break Struct](/nodes/structs/struct-break/) |
| Build an Array | [Make Array](/nodes/utils/array/make-array/) |
| Append an item | [Push](/nodes/utils/array/array-push/) |
| Read by index | [Get Element](/nodes/utils/array/array-get/) |
| Remove by index | [Remove Index](/nodes/utils/array/array-remove-index/) |

Do not assume a Blueprint object reference can be cast into a Flow-Like type.
Validate or transform incoming data with the node whose input contract matches
the source format.

## Variables and persistence

Flow variables are typed, board-level in-memory state. Read and write them with
Get Variable and Set Variable nodes, just as Blueprint getter and setter nodes
make state access visible.

Choose another store when the lifecycle is different:

| Requirement | Use |
| --- | --- |
| Temporary state used by the graph | Flow variable |
| Per-machine configuration or secret | [Runtime Variable](/apps/runtime-variables/) |
| Durable structured records | Database nodes |
| App-owned files | [App Storage](/apps/storage/) |
| Chat conversation state | Chat history and local/global sessions |

There is no Actor instance, replicated property, SaveGame object, or gameplay
framework behind a Flow variable.

## Events instead of gameplay callbacks

An event node begins execution inside a Flow. An [App Event](/apps/events/)
configures how that node is invoked.

| Automation need | Flow-Like entry |
| --- | --- |
| User clicks a named action | Simple Event node with a Quick Action |
| Recurring job | Simple Event node with a cron Event |
| HTTP request | Simple or Generic Event node with an API Event |
| Built-in conversation | Chat Event node with a Chat UI Event |
| Local application link | Compatible event node with a deeplink Event |
| Page or Widget interaction | UI action targeting an Event |

There is no equivalent to Event Tick. A cron Event is a scheduled automation,
not a per-frame callback. If work must wait or poll, use an explicit bounded
loop, Delay, timeout, or an external event rather than simulating a frame loop.

## What does not transfer

| Unreal capability | Flow-Like status |
| --- | --- |
| Actors, Pawns, Components, and World | No equivalent |
| Rendering and materials | Not a rendering engine |
| Physics, collision, overlap, and traces | No equivalent |
| Gameplay input | Use App interfaces and Events instead |
| Replication and network roles | Use ordinary API, authorization, and data contracts |
| Frame-rate-sensitive behavior | Not a real-time simulation workload |
| Gameplay Ability System | No direct equivalent |

Flow-Like Pages and Widgets can present workflow-backed interfaces, but they are
application UI rather than Unreal UMG or Slate widgets.

## Example: turn a gameplay-style monitor into automation

A Blueprint developer might recognize a “read state, compare, act, record”
pattern. A Flow-Like service monitor can implement it without a pseudo graph:

| Stage | Flow-Like implementation |
| --- | --- |
| Trigger | Cron Event every approved interval |
| Read | API Call to the metrics endpoint |
| Parse | Schema-constrained JSON or Struct |
| Compare | Typed comparison into Branch |
| Act | Notification node on the true path |
| Record | Database write containing status, time, and run ID |
| Recover | Bounded retry only for safe, repeatable requests |

This is event-driven work. It should not continuously poll at frame frequency,
and notifications should use an idempotency rule so a retry cannot send the
same alert repeatedly.

## Debugging and versions

Use [run history and logs](/studio/logging/) to inspect completed executions,
timing, and node output. This is not Blueprint's live gameplay debugger, so
design logs around stable run and correlation identifiers.

Create a saved Flow version after a behavior is verified. App Events can target
that version rather than the mutable latest graph; see
[Versioning](/studio/versioning/).

## Tips for Blueprint developers

1. Define pin types and schemas before arranging the graph.
2. Keep execution ordering explicit.
3. Use Functions for callable logic and Layers for visual abstraction.
4. Replace Tick-driven thinking with App Events.
5. Store durable state in a database or App Storage, not only in variables.
6. Check whether a node is local-only before selecting remote execution.
7. Validate every external payload as untrusted input.

## Next steps

- [Studio overview](/studio/overview/)
- [Nodes](/studio/nodes/)
- [Connections](/studio/connecting/)
- [Layers](/studio/layers/)
- [Variables](/studio/variables/)
- [Events](/apps/events/)
- [Pages and A2UI](/apps/a2ui/)
