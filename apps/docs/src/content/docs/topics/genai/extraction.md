---
title: Extraction & Structured Output
description: Extract schema-validated JSON from text, chat history, and documents
sidebar:
  order: 6
---

Flow-Like's AI extractors use a configured model and a runtime schema to turn free-form content into structured JSON. The model must return the extraction through a required tool call, and the result is validated against the schema before the node succeeds.

## Extraction nodes

| Input | Node |
|-------|------|
| Free-form text | [AI Extractor](/nodes/ai/generative/llm-extractor/) |
| Model-compatible conversation history | [AI Extractor from History](/nodes/ai/generative/llm-extractor-history/) |
| One document requiring text and image-aware extraction | [AI Extract Document](/nodes/ai/processing/ai-processing-extract-document-ai/) |
| Several documents | [AI Extract Documents](/nodes/ai/processing/ai-processing-extract-documents-ai/) |
| Predictable document content without AI enhancement | [Extract Document](/nodes/ai/processing/ai-processing-extract-document/) |

The AI Extractor nodes accept:

- a configured model;
- a JSON Schema or example JSON;
- the text or history;
- an optional extraction hint.

They return the validated JSON value plus model usage statistics. The configured model must support the required tool or function call behavior.

## Choose the extraction boundary

| Need | Recommended boundary |
|------|----------------------|
| Classify one message | Extract a small enum and confidence explanation fields |
| Parse a ticket or email | Extract the normalized fields from the message body |
| Process a scanned invoice | Extract document text, then extract the invoice schema |
| Summarize a conversation into CRM fields | Use the history extractor |
| Extract repeated rows | Use an array item schema and validate each item |
| Pull known fields from clean JSON | Use deterministic JSON parsing instead of a model |

Use deterministic parsing, regular expressions, or native document nodes when the source has a reliable contract. Model extraction is most useful when wording or layout varies.

## Define the schema

### JSON Schema

A JSON Schema gives direct control over required fields, enums, nullability, and nested objects:

```json
{
  "type": "object",
  "properties": {
    "ticket_id": {
      "type": "string",
      "description": "The support ticket identifier exactly as written"
    },
    "priority": {
      "type": "string",
      "enum": ["low", "medium", "high", "urgent"]
    },
    "customer_email": {
      "type": ["string", "null"],
      "description": "Customer email, or null when none is present"
    },
    "summary": {
      "type": "string"
    }
  },
  "required": ["ticket_id", "priority", "customer_email", "summary"],
  "additionalProperties": false
}
```

### Example JSON

The extractor also accepts example JSON and infers a schema from it:

```json
{
  "vendor": "Example Ltd.",
  "invoice_number": "INV-1007",
  "currency": "EUR",
  "total": 1280.5,
  "line_items": [
    {
      "description": "Consulting",
      "quantity": 8,
      "unit_price": 160
    }
  ]
}
```

Use an explicit JSON Schema for production contracts. An example is convenient for prototyping but cannot express every validation rule or missing-value decision.

## Schema design

### Make missing data explicit

If a field may be absent in the source, allow `null` and describe when it should be used. Do not ask the model to guess a value to satisfy a required field.

### Constrain known categories

Use `enum` for a controlled set such as status, priority, or document type. Add a safe fallback category only when the downstream process can handle it.

### Describe source fidelity

Field descriptions should say whether to:

- copy text exactly;
- normalize casing or whitespace;
- convert a date or amount;
- infer from context;
- return null when unsupported.

### Keep the first schema small

Start with the fields needed for the next operation. Deeply nested schemas make failures harder to diagnose and increase the chance that the model fills gaps with plausible-looking values.

### Use stable types

Decide whether identifiers are strings or numbers, how currencies are represented, and which date format downstream nodes expect. Do not change types based on what one example happens to contain.

## Build an extraction workflow

1. read or extract the source content;
2. retain a source identifier and location;
3. select the configured model;
4. pass the schema, content, and a narrow hint to the extractor;
5. validate business rules that JSON Schema cannot express;
6. route invalid or uncertain results to review;
7. store the structured data with provenance and model configuration.

For a document, use the native or AI-assisted document reader first. The schema extractor works on text or history, while the document reader handles the file format and visual content.

## Validate the result

Schema validation confirms shape, not truth. Add deterministic checks:

- totals reconcile with line items;
- dates fall in an acceptable range;
- identifiers match the expected pattern;
- email or URL syntax is valid;
- enum combinations are permitted;
- required source evidence exists;
- duplicate records are detected;
- high-impact fields are confirmed by a reviewer when needed.

Preserve the original value when normalization could make an audit difficult.

## Extraction hints

Use the optional hint for one narrow instruction that complements the schema, such as:

- extract only individual line items, not subtotal rows;
- use the email sender as the customer only when the body does not name one;
- keep all quoted legal text verbatim;
- classify from the supplied categories without creating new ones.

Do not duplicate a long system prompt in the hint. Put constraints in field descriptions and workflow validation where they can be reviewed precisely.

## Common patterns

| Pattern | Example fields |
|---------|----------------|
| Contact | name, organization, email, phone |
| Support ticket | category, priority, affected product, summary |
| Invoice | vendor, invoice number, dates, currency, line items, totals |
| Meeting | attendees, decisions, action items, owners, due dates |
| Sentiment | label, evidence excerpt, review flag |
| Document classification | document type, key identifiers, confidence explanation |

For subjective outputs such as sentiment, include the evidence excerpt or rationale field needed for review. Do not represent a model-generated probability as calibrated confidence unless it has been validated as such.

## Combine with other features

### Retrieval

Retrieve a small set of relevant passages, then extract a schema from those passages. Keep source references attached so each structured field can be audited. See [RAG and knowledge bases](/topics/genai/rag/).

### Agents

Expose extraction as a bounded function tool when an agent may choose to structure content. The tool should still validate schema and authorization outside the model. See [AI agents](/topics/genai/agents/).

### Chat

Use **AI Extractor from History** to convert an intake conversation into typed workflow data. Ask a structured follow-up when a required field is missing instead of inventing it. See [Chat and conversations](/topics/genai/chat/).

## Privacy and operations

- Send only necessary source content to the configured model.
- Redact or mask sensitive fields before extraction when the task permits.
- Record provider, model, schema version, and usage with the run.
- Do not log entire documents or extracted personal data by default.
- Define retention for source files and structured outputs.
- Keep a review path for high-impact or low-quality records.

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Model does not return structured data | Tool/function-call support, model configuration |
| Schema input fails | Valid JSON, valid JSON Schema, non-empty input |
| Missing fields | Source evidence, nullability, descriptions, schema size |
| Extra invented fields | `additionalProperties`, field descriptions, validation |
| Wrong types | Explicit schema types and normalization rules |
| Inconsistent values | Enum constraints, examples, deterministic business checks |

## Next steps

- [Document processing](/topics/document-processing/overview/)
- [RAG and knowledge bases](/topics/genai/rag/)
- [AI agents](/topics/genai/agents/)
- [Chat and conversations](/topics/genai/chat/)
