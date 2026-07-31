---
title: For n8n Users
description: Import an n8n workflow and adapt it to Flow-Like's typed execution model
sidebar:
  order: 4
---

Flow-Like and n8n both use connected workflow graphs, but their data,
configuration, and execution models are different. Flow-Like includes a
clipboard importer for n8n workflow JSON; use it as a migration starting point,
then validate every imported node and boundary.

## Import an n8n workflow

1. Export the workflow as JSON from n8n.
2. Create or open the destination Flow in Flow-Like Studio.
3. Copy the complete exported JSON.
4. Place the pointer on an empty area of the Flow canvas and paste.
5. Review the imported nodes, layers, variables, defaults, and connections.

The importer recognizes the n8n `nodes` and `connections` structure. It places
the translated graph at the paste location and uses the current Flow-Like node
catalog when a mapping exists.

:::caution[Import is a translation, not a deployment]
The import does not transfer live credentials, create all App Events, or prove
that source and destination semantics are identical. Run representative test
payloads before enabling an imported Flow.
:::

## What the current importer maps

The current repository includes mappings for selected workflow primitives,
integrations, and model nodes.

| n8n source | Imported Flow-Like shape | Required review |
| --- | --- | --- |
| Manual, schedule, webhook, and chat triggers | Event-node candidates | Create the matching App Event and verify its payload |
| HTTP Request | Layer containing request construction, API Call, and response conversion | Headers, authentication, body, response field, and error behavior |
| IF | Branch | Rebuild the source condition from typed values |
| Switch | Branch | Multi-way routing must be rebuilt with additional branches |
| Split In Batches | For Each | Batch size, loop completion, and retry behavior |
| Set | Set Field | Field paths, types, and retained input fields |
| Wait | Delay | Unit, duration, and resume expectations |
| Merge or No Op | Sequence-style placeholder | Ordering and actual merge semantics |
| Code | Python Interpreter-style node | Convert JavaScript manually and review its permissions |
| Gmail | SMTP connection and send layer | Host, port, message fields, and credentials |
| Google Sheets, Discord, and Telegram | Catalog mapping where available | Operation-specific inputs and authorization |
| Selected chat-model nodes | Model-builder mapping where available | Model selection and credentials |

Unsupported node types become named **TODO** layers with placeholder nodes.
Do not delete those markers until the missing behavior has been rebuilt or
intentionally removed.

Sticky notes become Flow comments. Disabled n8n nodes are skipped.

## Credentials after import

n8n credential references do not contain the secret value, and the importer
does not attempt to fetch it. For each imported credential reference it creates
a secret Flow variable and, when possible, wires a Get Variable node to a
matching authentication pin.

After import:

1. inspect every generated credential variable;
2. rename it if several credentials need clearer identities;
3. enable **Runtime Configured**;
4. set the value through the App's
   [Runtime Variables](/apps/runtime-variables/) screen;
5. confirm that no token or password remains in an ordinary node default;
6. configure the value independently on every machine or environment that will
   execute the Flow.

## Translate the mental model

| n8n concept | Flow-Like concept |
| --- | --- |
| Workflow | Flow |
| Project boundary | App |
| Node | Node |
| Connection | Execution or data wire |
| Trigger node | Event node plus an App Event |
| Execution | Run |
| JSON item | Typed value, often a Struct |
| List of items | Typed Array |
| Expression | Explicit data-access, transform, and math nodes |
| Credential | Secret, runtime-configured variable |
| Sub-workflow | Flow function, Layer, or separately invoked Flow |
| Sticky note | Comment |

The important difference is data flow. n8n commonly passes JSON items through a
node. Flow-Like pins carry declared types, and a node receives only the values
wired to its inputs.

## Replace expressions with typed operations

n8n expressions can combine lookup and transformation in one field:

```javascript
{{ $json.customer.name }}
{{ $json.items[0].price * $json.items[0].quantity }}
```

In Flow-Like, keep those steps explicit:

| Expression step | Current node |
| --- | --- |
| Read `customer.name` or `items[0].price` | [Get Field](/nodes/structs/fields/struct-get/), which supports dot notation and array access |
| Multiply numeric values | Typed Math node |
| Insert values into text | [Render Template](/nodes/utils/string/string-render-template/) or Format String |
| Parse JSON with a known shape | [Parse JSON with Schema](/nodes/utils/json/parse-with-schema/) |

This adds nodes, but it also makes type mismatches and data dependencies visible
before the Flow reaches an external side effect.

## Items and loops

n8n's item propagation does not translate to implicit repeated execution.
When a Flow-Like node returns an Array, choose the intended collection behavior:

| Intent | Flow-Like node |
| --- | --- |
| Process each item in order | [For Each](/nodes/control/control-for-each/) |
| Stop early | [For Each (Break)](/nodes/control/control-for-each-with-break/) |
| Process items concurrently | [Parallel For Each](/nodes/control/control-par-for-each/) |
| Retrieve an item by index | [Get Element](/nodes/utils/array/array-get/) |
| Add an item | [Push](/nodes/utils/array/array-push/) |
| Query tabular data | DataFusion SQL rather than a graph-sized loop |

Check empty arrays, ordering, duplicate handling, and concurrency limits
explicitly.

## Triggers become App Events

An imported trigger node is only the entry point inside the Flow. Configure an
[App Event](/apps/events/) to invoke it.

| Desired entry | Flow node and Event configuration |
| --- | --- |
| User-initiated action | Simple Event node with a Quick Action |
| Schedule | Simple Event node with a cron Event |
| HTTP endpoint | Simple or Generic Event node with an API Event, depending on the required payload |
| Built-in chat | Chat Event node with a Chat UI Event |
| Local deep link | Compatible event node with a deeplink Event |
| Long-running local process | Simple Event node with a daemon Event |

Event availability depends on local versus remote execution. API and cron
Events can be configured for supported local or remote execution; deep links
and daemon Events are local, while REST and MCP server Events are remote. See
[Offline versus online](/apps/offline-online/) before choosing the location.

## Reusable workflows

Do not assume an n8n Execute Workflow node imported successfully. For reusable
logic inside one Flow, define a typed Flow function and call it with
[Call Function](/nodes/control/functions/control-call-function/). Use a
[Layer](/studio/layers/) when the main goal is to collapse and name a graph
section.

When a separately deployed Flow is the correct boundary, give it its own event
contract and invoke that contract deliberately. Preserve correlation IDs,
timeouts, authorization, and failure handling across the boundary.

## Example: migrate a webhook workflow

Suppose the source receives an order, enriches it through an API, branches on
the result, and sends a notification. Rebuild and verify it in this order:

| Stage | Migration action |
| --- | --- |
| Entry | Use a Generic or Simple Event node and define the expected payload |
| Exposure | Configure an API Event with the required method and path |
| Validation | Parse or constrain the incoming Struct before using its fields |
| Enrichment | Build the request, perform the API Call, and validate status and response |
| Decision | Feed an explicit Boolean condition into Branch |
| Notification | Use a catalog integration or a narrowly configured HTTP call |
| Failure | Return or log a clear failure without implying the notification succeeded |

Use a test endpoint until both the success and failure paths match the source.
If the notification is not safe to repeat, add an idempotency check before
retrying.

## Post-import checklist

- [ ] Every TODO layer is resolved or intentionally removed
- [ ] Every imported warning or review comment is addressed
- [ ] Trigger nodes have matching App Events
- [ ] Event payloads and response behavior are defined
- [ ] Credentials are runtime configured
- [ ] JavaScript Code nodes are manually converted and reviewed
- [ ] Arrays, loops, ordering, and empty-input behavior are tested
- [ ] External writes are idempotent or protected from duplicate runs
- [ ] Local-only nodes use a compatible execution mode
- [ ] A verified Flow version is pinned before production use

## Next steps

- [Studio overview](/studio/overview/)
- [Events](/apps/events/)
- [Runtime Variables](/apps/runtime-variables/)
- [Typed connections](/studio/connecting/)
- [API integrations](/topics/api-integrations/overview/)
- [Run history and logs](/studio/logging/)
