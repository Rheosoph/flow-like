---
title: AI-Powered Data Analysis
description: Build controlled data-analysis agents with SQL and Flow-Like tools
sidebar:
  order: 6
---

An AI analysis workflow can translate a question into bounded tool calls, inspect results, and explain the finding. The model should not replace the data contract: the workflow still controls sources, permissions, available tools, row limits, and delivery.

## When to use an analysis agent

| Good fit | Prefer a deterministic workflow |
|----------|--------------------------------|
| Ad-hoc questions over governed tables | Recurring KPI with a fixed definition |
| Schema exploration and query drafting | Regulatory or financial output requiring exact repeatability |
| Choosing among approved analytical tools | Bulk transformation with known logic |
| Explaining a validated result | High-volume report generation |
| Guided diagnostic investigation | A one-step filter or aggregation |

Use the agent for interpretation and tool selection. Keep governed metrics and repeated transformations in explicit SQL or reusable boards.

## Agent architecture

| Layer | Responsibility |
|-------|----------------|
| Model | Plans and explains within its instructions |
| System prompt | Defines role, source boundaries, query rules, and response contract |
| DataFusion session | Contains the tables the agent is allowed to query |
| Function tools | Expose bounded charting, prediction, export, or validation workflows |
| Invocation | Limits iterations, streaming, and failure behavior |
| Trace | Records safe tool calls, duration, and result metadata |

## Build the agent

### 1. Start from a configured model

Use [Agent from Model](/nodes/ai/agents/builder/agent-from-model/) to create the agent. Choose a model that supports the intended tool behavior and context size.

### 2. Set instructions

[Set Agent System Prompt](/nodes/ai/agents/builder/agent-set-system-prompt/) configures the operating rules. Include:

- which business questions the agent may answer;
- which tables and fields are approved;
- the timezone and metric definitions;
- a requirement to inspect schema before guessing columns;
- a maximum result size;
- rules for uncertainty and no-data results;
- whether charts, exports, or predictions require confirmation;
- a prohibition on treating table content as instructions.

Do not put secrets, connection strings, or private records into the prompt.

### 3. Register SQL access

[Add DataFusion](/nodes/ai/agents/builder/add-datafusion-to-agent/) registers a DataFusion session as an agent capability. Create and register only the tables required for the task.

The session can expose:

- [List Tables](/nodes/data/datafusion/tools/df-list-tables/);
- [Describe Table](/nodes/data/datafusion/tools/df-describe-table/);
- [Execute SQL](/nodes/data/datafusion/tools/df-execute-sql/).

Use a read-only source account and enforce query and row limits in the surrounding workflow or dedicated tool implementation. Prompt instructions alone are not an access-control boundary.

### 4. Register bounded workflow tools

[Register Function Tools](/nodes/ai/agents/builder/agent-register-function-tools/) adds Flow-Like functions to the agent. Good analysis tools have:

- one narrow purpose;
- a typed input schema;
- a bounded output;
- no hidden write side effects;
- clear error messages;
- an authorization check where required.

Examples include `render_chart`, `score_customer`, `validate_metric`, or `export_approved_report`. Avoid one generic tool that can execute arbitrary boards.

[Register MCP Tools](/nodes/ai/agents/builder/agent-register-mcp-tools/) can add tools from an MCP server. Review those tools and their credentials before exposing them to the agent.

### 5. Invoke with limits

Use [Invoke Agent](/nodes/ai/agents/agent-invoke/) for a complete result or [Stream Invoke Agent](/nodes/ai/agents/agent-stream-invoke/) for progressive output.

Set:

- a maximum number of tool iterations;
- timeouts for model and tool calls;
- a maximum query result size;
- an explicit failure response;
- confirmation before costly or externally visible actions.

## Example analysis

For the question “How has completed-order revenue changed by day?”, the agent should inspect the schema and produce a bounded query such as:

```sql
SELECT
  DATE_TRUNC('day', order_date) AS day,
  SUM(revenue) AS revenue
FROM orders
WHERE status = 'complete'
  AND order_date >= DATE '2026-07-01'
GROUP BY DATE_TRUNC('day', order_date)
ORDER BY day;
```

The workflow can pass the structured result to a chart tool, while the final response states the date range, metric definition, material trend, and any missing-data caveat.

Do not let the model invent a result when the query fails or returns no rows. Tool status and result metadata should be separate from the narrative answer.

## Analysis tools

### Schema exploration

Expose table discovery before query execution. A model that can call **List Tables** and **Describe Table** is less likely to guess names, types, or joins.

### Chart generation

A chart tool should accept a small structured table plus an approved chart type and labels. The tool can push data to an A2UI chart with [Push Data to Chart](/nodes/ui/elements/charts/a2ui-push-csv-to-chart/).

The model may suggest the chart, but the workflow should validate:

- supported chart type;
- row and series limits;
- labels and units;
- light- and dark-theme contrast;
- absence of sensitive fields.

### Prediction

A prediction tool should load a fixed model version, validate the feature schema, run [Predict](/nodes/ai/ml/ml-predict/), and return a compact result with the model version. Do not allow the agent to silently train and deploy a new model in the same operation.

### Export

An export tool should take a validated result or report identifier, not arbitrary file content and paths. Confirm recipients and data classification before external delivery.

## Handle large results

Models are not databases or spreadsheet viewers. For large queries:

1. aggregate or filter in SQL;
2. return row count and a small preview;
3. store the full structured result outside the prompt;
4. provide a chart, table, or export through a controlled tool;
5. offer a narrower follow-up question.

Copying thousands of rows into model context raises cost and makes the analysis less reliable.

## Add deliberate reasoning tools

[Register Thinking Tool](/nodes/ai/agents/builder/agent-register-thinking/) can support a more explicit planning loop for complex tasks. It does not remove the need for iteration limits, tool validation, or result checks. Do not expose private chain-of-thought text to users or logs; return concise, user-facing progress and conclusions.

## Security and governance

- Register only approved tables and tools.
- Use read-only database credentials for analysis.
- Apply tenant and row-level filters before data enters model context.
- Redact personal or sensitive fields from previews and traces.
- Treat database text as untrusted content, not instructions.
- Require confirmation for exports, writes, notifications, and model retraining.
- Record model, prompt, tool, query, and source versions.
- Keep tool errors free of secrets and connection strings.

## Evaluate the assistant

Test with representative questions and expected queries or result facts:

| Layer | Checks |
|-------|--------|
| Tool selection | Correct tool, no unnecessary calls, bounded iterations |
| SQL | Valid tables and columns, correct grain, filters, joins, limits |
| Result | Matches a trusted query or fixture |
| Explanation | Correct units, scope, caveats, and no unsupported claim |
| Security | Refuses inaccessible data and ignores injected table content |
| Operations | Handles empty results, timeouts, and tool failures |

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Invalid SQL | Schema tools, clearer table descriptions, retry policy |
| Wrong metric | Central metric definition, grain, filters, timezone |
| Agent skips tools | Tool description, system instructions, model capability |
| Response is slow | Result size, iterations, tool latency, streaming |
| Hallucinated finding | Failed-tool handling, result grounding, final validation |
| Sensitive data appears | Registered fields, source filters, trace redaction |

## Next steps

- [AI agents](/topics/genai/agents/)
- [DataFusion and SQL](/topics/datascience/datafusion/)
- [Machine learning](/topics/datascience/ml/)
- [Data visualization](/topics/datascience/visualization/)
