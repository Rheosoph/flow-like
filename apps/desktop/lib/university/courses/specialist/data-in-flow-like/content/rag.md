Ask a bare model "What's the refund window for annual plans?" and it answers instantly, fluently — and quite possibly wrongly, because it has never seen your `refund-policy.md`. Retrieval-Augmented Generation fixes that: find the relevant passages first, then make the model answer from them. The whole mental model fits in one picture.

@RAGOverview

Two separate lanes. **Index once** (Documents → Chunk with metadata → Embed → a searchable knowledge index) runs when a source is added or changed. **Answer each question** (User question → Embed with the *same model* → Retrieve → Add context → grounded answer with sources) runs per question, drawing on the index the first lane built. The caption under the diagram is the philosophy in one line: retrieval adds evidence to the prompt; the model still generates the final response. Put differently — RAG is a data pipeline with a model at the end.

## 1 · Index once

For the copilot, the index lane consumes `refund-policy.md` and `support-playbook.pdf` from Storage:

1. **Extract** with the right document node, keeping source ID, title, page or section, version, and access metadata.
2. **Chunk** without breaking tables or procedures mid-thought. Small chunks sharpen precision, large ones keep context, overlap protects boundaries at the cost of duplication.
3. **Embed** every chunk with one fixed model configuration.
4. **Store** text + vector + provenance in a native table, with deterministic chunk IDs derived from source version and location — lesson 3's identity discipline, applied to chunks. Flush before declaring the index ready.
5. **Reconcile versions:** a new policy version replaces or retires every old chunk for that source, and deleting a source deletes every chunk derived from it.

## 2 · Answer each question

At question time: embed with the same model and configuration, retrieve candidates, apply metadata and access filters, select the best passages — deduplicate, stay inside the context budget — then prompt for an answer grounded only in the supplied context, with citations, and an explicit "the corpus doesn't say" when evidence is missing.

Two hard rules. First, the caller's authorization runs *before* retrieved text reaches the model — no post-generation redaction can unsay what the model already read. Second, retrieved documents are untrusted data: a passage may literally contain instructions to the model. Delimit evidence in the prompt and tell the model to treat it as content, but enforce access and source selection outside the model.

## 3 · Pick the search mode

**Full-text search** is exact: policy codes, product names, "REF-201". **Vector search** matches meaning: "money back on annual plans" finds the refund section even though no word matches. **Hybrid search** runs both and can fuse the candidate lists with reciprocal rank fusion. Support agents ask both ways in the same shift, so hybrid with metadata filters — version, document type, tenant — is the strong default.

## 4 · Prove it works

Build a small test set before tuning anything: a known-answer question, an exact code, a paraphrase, an answer spanning adjacent chunks, a no-answer question that must produce an explicit insufficiency response, conflicting old and new versions, one prompt-injection passage, and one source the test user can't access. Then measure retrieval separately from answers: did the right chunk arrive (hit rate, rank, duplicate rate) versus was the answer faithful (citation correctness, unsupported claims). A fluent answer built on missed retrieval is still a failure — it just fails politely.

**Watch out:** more context is not better context. Irrelevant passages dilute the evidence and crowd the budget the good passages need.

## Recap

- Two lanes: index on change, answer per question — never one merged workflow.
- Same embedding config in both lanes, deterministic chunk IDs, provenance on every chunk, access filters before generation.
- Evaluate retrieval and answers separately, with no-answer, stale-version, injection, and access cases in the suite.
