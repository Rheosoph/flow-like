Monday night the runbook re-index timed out halfway, and the pipeline retried. Tuesday morning every answer cites the gateway runbook twice, and a question about the on-call rota returns last year's rotation above the current one. Nothing crashed; every node reported success. Before reading on, name the two invariants this pipeline is missing. Most RAG failures that look like "the model ignored the document" are born right here, in the index.

## 1 · The pipeline

@RagArchitecture

You're building the top lane of the diagram — Documents → Chunk → Embed → Knowledge index — as its own Flow, separate from the answer lane. Five stages, each with a signature failure:

| Stage | What it does | What goes wrong |
| --- | --- | --- |
| Discover | Identify the source and its version | A file name treated as identity |
| Extract | Pull text and structure with document nodes | Navigation menus instead of body text |
| Chunk | Split on meaning-preserving boundaries | Tables severed from their headers |
| Embed | Vectorize with one recorded model config | Query model drifts from index model |
| Store | Stage, validate, then switch versions | Old chunks outranking the new version |

Keep this Flow separate from the query Flow: corpus updates shouldn't couple to live questions, and you can evaluate retrieval without paying for generation on every test.

## 2 · Identity makes retries safe

Here's Monday's diagnosis. `source_id` comes from the authoritative system, `source_version` from a revision or content hash, and every `chunk_id` derives deterministically from source, version, and location — like `runbook-gateway-v3-rotation-02` from the last lesson. With stable IDs, storing is an upsert: a retry rewrites the same records instead of minting duplicates. That's missing invariant number one.

Number two is retirement. Build the new version completely, validate its record and vector counts, then switch — and retire every chunk of the old version. Updates that merely append leave last year's rota and this year's competing in one result list, and the model can't referee that fight. For deletion, remove everything matching the stable source ID. For an embedding-model change, build a separate index and switch queries deliberately; vector spaces never mix in one search path.

## 3 · Chunk by meaning

Flow-Like gives you **Chunk Text** (`chunk_text`), a splitter tuned to the configured embedding model, and **Character Chunk Text** (`chunk_text_char`) for manual control. There's no universal chunk size: smaller chunks sharpen matches but strand context, bigger chunks keep relationships but dilute similarity, and overlap protects boundary facts while inflating the index. Whatever you tune, protect meaning: keep a table's header with its rows, keep numbered steps with their preconditions, copy the section heading into each child chunk's metadata, and keep page or cell references for citations.

## 4 · Prove it before you query it

Use **Embed Document** (`embed_document`) for chunks, store text, vector, and provenance together, and interrogate the index before any agent exists. Write ten *golden questions* with known source passages: a paraphrase, an exact identifier, a fact spanning two chunks, a restricted document, an updated document, and one question with no answer. Run semantic, full-text, and hybrid test queries and check that the expected chunk appears — and at what rank. If it never appears, generation can't save you: go fix extraction, boundaries, or embedding compatibility first.

> **Watch out:** don't log raw document text to prove the pipeline ran. Counts, versions, IDs, and timings tell the story without leaking the corpus.

## Recap

- Deterministic chunk IDs plus upsert make retries idempotent.
- Version switch-and-retire keeps old truth from outranking new truth.
- Golden questions validate retrieval before generation exists.
