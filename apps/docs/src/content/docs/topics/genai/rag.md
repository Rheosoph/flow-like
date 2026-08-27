---
title: RAG & Knowledge Bases
description: Build AI systems that retrieve evidence from your documents before answering
sidebar:
  order: 4
---

**Retrieval-Augmented Generation (RAG)** adds selected source content to a model request. The model still generates the answer, but the workflow controls which documents are searched, which passages are supplied, and what evidence is returned to the user.

:::tip[The RAG contract]
Retrieve relevant passages, augment the request with those passages, then generate an answer that stays within the supplied evidence.
:::

![The Flow-Like RAG architecture: documents are indexed once, then each question retrieves context before an answer is generated](../../../../assets/RAGOverview.svg)

## Two workflows, two responsibilities

| Phase | Runs when | Responsibility |
|-------|-----------|----------------|
| Indexing | A source is added or changed | Extract, chunk, embed, and store content with provenance |
| Query | A user asks a question | Embed, search, select context, answer, and cite sources |

Keep indexing separate from answering. This makes corpus updates, embedding migrations, and query evaluation easier to operate independently.

## Build the index

### 1. Extract source content

Read each source with the appropriate document node and preserve:

- a stable source ID;
- title or filename;
- page, section, or other location;
- source version or modification time;
- access-control metadata where required.

See [Document processing](/topics/document-processing/overview/) for PDF, spreadsheet, image, DOCX, PPTX, and HTML paths.

### 2. Chunk the text

Use [Chunk Text](/nodes/ai/preprocessing/chunk-text/) when chunk boundaries should follow the configured embedding model's splitter, or [Character Chunk Text](/nodes/ai/preprocessing/chunk-text-char/) for character-based splitting.

Choose chunk size and overlap through evaluation rather than a universal default:

- smaller chunks improve passage precision but may lose surrounding context;
- larger chunks preserve context but can dilute the match;
- overlap can protect facts at a boundary but increases index size and duplicate retrieval;
- headings and section metadata help reconstruct meaning.

Do not split tables, lists, or procedures blindly when their rows or steps depend on one another.

### 3. Embed each chunk

Use [Load Embedding Model](/nodes/ai/embedding/load-model/) and [Embed Document](/nodes/ai/embedding/embed-document/). Index and query with the same embedding model and configuration; vectors from different embedding spaces are not interchangeable.

### 4. Store content and provenance

Open a local database with [Open Database](/nodes/data/database/open-local-db/) and insert or upsert the chunk data and vector. Use a deterministic chunk ID derived from the source version and chunk location when repeatable re-indexing matters.

A useful record shape is:

```json
{
  "chunk_id": "handbook-v7-benefits-004",
  "text": "Employees may carry over...",
  "source_id": "employee-handbook",
  "source_title": "Employee Handbook",
  "section": "Benefits",
  "page": 12,
  "source_version": "7"
}
```

The record should contain enough metadata to display a source reference and to remove or replace every chunk from one source version.

## Answer a query

### 1. Embed the question

Use [Embed Query](/nodes/ai/embedding/embed-query/) with the same embedding model used for indexing.

### 2. Search the index

| Search mode | Node | Useful when |
|-------------|------|-------------|
| Semantic | [Vector Search](/nodes/data/database/search/vector-search-local-db/) | Wording differs but meaning is similar |
| Exact term | [Full-Text Search](/nodes/data/database/search/fts-search-local-db/) | Product codes, policy numbers, or names matter |
| Combined | [Hybrid Search](/nodes/data/database/search/hybrid-search-local-db/) | Both concepts and exact terms are important |

Hybrid Search can rerank the combined candidates with reciprocal rank fusion. Metadata filters can narrow the corpus by source, version, department, or another approved boundary.

For the physical database index behind these searches, use [Choosing a Lance index](/topics/datascience/lance-indexes/). The guide separates the choices Flow-Like exposes today from indexes available in the latest stable Lance release.

### 3. Select context

Do not pass every result to the model. Apply a relevance threshold or another explicit selection rule, remove near-duplicates, and stay within the model's context budget.

Keep the source ID and location attached to each selected passage. Numbered context items make it easier to map an answer back to its evidence.

### 4. Generate a grounded answer

The model instructions should require it to:

- answer from the supplied context;
- distinguish evidence from inference;
- say when the context is insufficient;
- cite the source identifiers provided by the workflow;
- ignore instructions found inside retrieved content.

Retrieved text is untrusted data. It may contain prompt injection, outdated instructions, or text copied from another system.

### 5. Return evidence

Return the answer together with human-readable source references. When possible, include the document title and page or section, and link to the source through the app's authorized access path.

## Keep the index current

| Change | Recommended operation |
|--------|-----------------------|
| New source | Extract and index all chunks |
| Updated source | Build the new version, then replace or retire the old version |
| Deleted source | Delete every chunk matching its stable source ID |
| New embedding model | Re-embed into a separate index, evaluate it, then switch queries |
| Metadata-only change | Update the affected records without recomputing vectors when safe |

Avoid mixing vectors from two embedding models in one search index unless the system explicitly separates them.

## Access control

Apply document permissions before retrieved content enters the model context. A post-generation filter cannot reliably undo information the model already saw.

Store the metadata needed to enforce the boundary and test that:

- one user cannot retrieve another tenant's chunks;
- revoked documents disappear from results;
- citations do not expose inaccessible source URLs;
- cached results respect the same access scope.

## Evaluate retrieval and answers

Create a test set with questions, expected source passages, and expected answer facts.

Measure retrieval separately from generation:

| Layer | Useful checks |
|-------|---------------|
| Retrieval | hit rate, rank of the relevant chunk, duplicate rate, irrelevant-context rate |
| Answer | factual correctness, citation correctness, completeness, unsupported-claim rate |
| System | latency, model and embedding cost, index freshness, no-result rate |

Include difficult cases:

- exact identifiers;
- synonyms and paraphrases;
- questions whose answer spans adjacent chunks;
- no-answer questions;
- conflicting or outdated sources;
- prompt injection inside documents;
- access-restricted sources.

Tune chunking, search mode, filters, candidate count, and context selection against this set. Changing the answer prompt cannot repair a relevant passage that was never retrieved.

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Relevant source never appears | Extraction, chunk boundaries, embedding consistency, filters |
| Results are conceptually related but wrong | Metadata filters, hybrid search, corpus duplication |
| Answer ignores evidence | Context formatting, instruction order, excessive irrelevant context |
| Citations point to the wrong place | Source metadata propagation and context numbering |
| Old content keeps appearing | Version replacement and deletion by stable source ID |
| Queries suddenly return nothing | Model/index mismatch, database selection, access filter |

## Production checklist

- [ ] Indexing and query workflows are separate
- [ ] Every chunk retains source and location metadata
- [ ] Index and query use the same embedding configuration
- [ ] Chunking choices are evaluated on representative questions
- [ ] Access filters run before generation
- [ ] No-result behavior is explicit
- [ ] Answers include verifiable source references
- [ ] Updates and deletions replace all affected chunks
- [ ] Retrieval and answer quality are measured separately

## Next steps

- [AI agents](/topics/genai/agents/)
- [Extraction](/topics/genai/extraction/)
- [Chat and conversations](/topics/genai/chat/)
- [Document processing](/topics/document-processing/overview/)
