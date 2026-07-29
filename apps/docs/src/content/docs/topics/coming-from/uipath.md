---
title: For UiPath Developers
description: Rebuild UiPath automations with Flow-Like Flows, Events, and local automation nodes
sidebar:
  order: 1
---

UiPath and Flow-Like both compose work visually, but Flow-Like is not an XAML
runtime and does not import UiPath projects. Treat migration as a controlled
rebuild: preserve the business contract, selectors, test cases, and operational
requirements while selecting current Flow-Like nodes.

## Translate the main concepts

| UiPath concept | Closest Flow-Like concept |
| --- | --- |
| Project or process | App containing one or more Flows |
| Workflow file | Flow |
| Activity | Node |
| Sequence | [Sequence](/nodes/control/control-sequence/) and connected nodes |
| Flowchart decision | [Branch](/nodes/control/control-branch/) |
| Argument | Typed event, function, or layer pin |
| Variable | Typed Flow variable |
| Library workflow | Flow function or Layer |
| Trigger | App Event targeting an event node |
| Job | Flow run |
| Asset or credential | Runtime-configured variable |
| Attended automation | Local Flow run from Desktop |
| Unattended automation | Local background Event or compatible remote Flow |
| Queue item | Explicit database record plus an Event or worker pattern |

There is no single Flow-Like component equivalent to UiPath Orchestrator. App
storage, roles, Events, Flow versions, execution modes, and a configured
self-hosted backend each cover part of that operational surface.

## Start with the right automation surface

Flow-Like's current Automation catalog separates targeting from reliability:

| Need | Catalog area |
| --- | --- |
| Automate a web page | [Browser nodes](/nodes/automation/browser/) |
| Control a native desktop application | [Computer nodes](/nodes/automation/computer/) |
| Locate controls through accessibility data | Computer accessibility nodes |
| Match visual templates or pixels | [Vision nodes](/nodes/automation/vision/) |
| Build reusable fallback selectors | [Selector nodes](/nodes/automation/selector/) |
| Match a target across UI changes | [Fingerprint nodes](/nodes/automation/fingerprint/) |
| Add timeout, retry, assertion, checkpoint, and recovery behavior | [RPA nodes](/nodes/automation/rpa/) |
| Use model-assisted observation or healing | [Automation LLM nodes](/nodes/automation/llm/) |

Begin a desktop automation with
[Start Automation Session](/nodes/automation/automation-start-session/) and
release its resources with
[Stop Automation Session](/nodes/automation/automation-stop-session/). Keep the
cleanup path reachable after both success and failure.

Browser selectors are normally more stable than screen coordinates for web
content. Accessibility elements are normally more stable than pixels for native
controls. Use images, coordinates, fingerprints, or model assistance as
deliberate fallbacks.

## Map common activity families

### Control and reliability

| UiPath activity or pattern | Current Flow-Like choice |
| --- | --- |
| If | [Branch](/nodes/control/control-branch/) |
| For Each | [For Each](/nodes/control/control-for-each/) |
| For Each with early exit | [For Each (Break)](/nodes/control/control-for-each-with-break/) |
| While | [While Loop](/nodes/control/control-while-loop/) |
| Parallel | [Parallel Execution](/nodes/control/control-par-execution/) |
| Delay | [Delay](/nodes/control/delay/) |
| Retry Scope | [Retry Loop](/nodes/automation/rpa/rpa-retry-loop/) |
| Try Catch in a UI automation | [Try Catch](/nodes/automation/rpa/rpa-try-catch/) |
| Verify visual state | Assert Template Exists or Assert Color At Position |
| Save diagnostic state | [Take Snapshot](/nodes/automation/rpa/rpa-take-snapshot/) |

Keep retries bounded. Do not retry a click, submit, payment, email, or other
externally visible action until the Flow can determine whether the first
attempt already succeeded.

### Files, documents, and tabular data

| UiPath activity family | Current Flow-Like choice |
| --- | --- |
| Read or Write Text File | [Read to String](/nodes/data/files/content/read-to-string/) or [Write String](/nodes/data/files/content/write-string/) |
| File and directory operations | [Data/Files catalog](/nodes/data/files/) |
| PDF text extraction | [PDF Extract Text](/nodes/document/pdf/pdf-extract-text/) |
| Excel workbook operations | [Excel catalog](/nodes/data/excel/) |
| CSV or Parquet analytics | Mount or register the source in DataFusion |
| Filter or join tabular data | [DataFusion SQL](/nodes/data/datafusion/) |
| Persist structured records | Database nodes |

Do not translate every DataTable operation into a long graph of row mutations.
When the source is naturally tabular, register it and express filters, joins,
aggregations, and projections in SQL.

### APIs and messages

| UiPath activity family | Current Flow-Like choice |
| --- | --- |
| HTTP Request | Build a typed request and use [API Call](/nodes/web/api/http-fetch/) |
| JSON deserialize | [Parse JSON with Schema](/nodes/utils/json/parse-with-schema/) |
| JSON field access | [Get Field](/nodes/structs/fields/struct-get/) |
| SMTP send | [SMTP nodes](/nodes/email/smtp/) |
| IMAP mailbox access | [IMAP nodes](/nodes/email/imap/) |
| Service-specific API without a catalog node | Narrowly configured Web/API nodes |

Use Runtime Variables for tokens, passwords, environment-specific endpoints,
and other values that must not be stored in the Flow definition.

## Arguments, variables, and assets

UiPath arguments cross workflow boundaries. In Flow-Like, define typed pins on
the boundary that is actually being called:

- an event node for an App Event payload;
- a Flow function for reusable internal logic;
- a Layer for a collapsed graph section;
- a Page or Widget action when a user interface invokes the Flow.

Flow variables are board-level, in-memory state. They are not a credential
vault or a durable queue. For an Asset-like value, mark the variable **Runtime
Configured** and, for sensitive values, **Secret**, then configure it through
[Runtime Variables](/apps/runtime-variables/).

For durable work items, store records with explicit status, attempt count,
timestamps, correlation ID, and idempotency key. Trigger processing with an
appropriate Event. This makes queue semantics visible instead of treating every
Event as a queue.

## Attended and unattended execution

Desktop and browser automation nodes require a compatible local execution
environment and active operating-system session. A Flow containing any
local-only node must run locally; one run is not split between local and remote
workers.

| Scenario | Recommended shape |
| --- | --- |
| User starts a desktop task | Local Flow with a Quick Action |
| Local task runs on a schedule | Local cron Event while Desktop's event runner is available |
| Long-running local worker | Local daemon Event with bounded recovery |
| Server-side API or schedule | Remote-compatible Flow and remote Event |
| Team-managed online App | Online App with explicit roles and versioned Events |

See [Local-only execution](/studio/local-execution/) and
[Offline versus online](/apps/offline-online/) before choosing an execution
mode. A remote Event cannot execute desktop input, screen capture, or another
local-only node.

## Example: invoice processing

An invoice automation can preserve a typed extraction contract instead of
relying on fields scattered across activities.

Use a JSON Schema such as:

```json
{
  "type": "object",
  "properties": {
    "vendor": { "type": "string" },
    "invoice_number": { "type": "string" },
    "invoice_date": { "type": "string" },
    "line_items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "description": { "type": "string" },
          "quantity": { "type": "number" },
          "unit_price": { "type": "number" }
        },
        "required": ["description", "quantity", "unit_price"]
      }
    },
    "total": { "type": "number" }
  },
  "required": ["vendor", "invoice_number", "line_items", "total"]
}
```

Then build and test these stages:

| Stage | Flow-Like implementation |
| --- | --- |
| Receive the file | Quick Action, API Event, or App Storage input |
| Extract text | PDF Extract Text |
| Extract structured fields | [AI Extractor](/nodes/ai/generative/llm-extractor/) with the schema |
| Validate business rules | Typed comparisons and Branch nodes |
| Process line items | For Each |
| Persist | Database write with a stable invoice ID |
| Notify | SMTP or another approved integration |
| Handle failure | Log a safe run ID and retain the source for review |

AI schema conformance does not prove that an invoice value is correct. Validate
totals, identifiers, duplicate invoices, and required approvals before writing
to a financial system.

## Migration checklist

1. Inventory workflows, arguments, assets, selectors, queues, schedules, and
   unattended requirements.
2. Classify each automation as Browser, Computer, API, document, or data work.
3. Define typed input and output contracts before rebuilding activities.
4. Start with deterministic selectors and add bounded fallbacks.
5. Move secrets and environment values to Runtime Variables.
6. Model queue and retry state as explicit durable records.
7. Add assertions after consequential UI actions.
8. Test on the same operating system, permissions, theme, and display scaling
   used in production.
9. Configure Events and pin a verified Flow version.

## Next steps

- [Desktop and Browser Automation](/topics/desktop-automation/overview/)
- [Local-only execution](/studio/local-execution/)
- [Events](/apps/events/)
- [Runtime Variables](/apps/runtime-variables/)
- [Document processing](/topics/document-processing/overview/)
- [Self-hosting overview](/self-hosting/overview/)
