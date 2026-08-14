Ask your brand-new assistant "how do I rotate the payment gateway key?" and it will answer — fluently, confidently, and from nothing but training data. It has never seen your runbooks. The rotation procedure it describes belongs to some other company's stack, if anyone's.

> **Predict first:** if you hand the model your documents, does it become *truthful*? Hold your answer — this lesson tests it.

## 1 · Index once, retrieve on every question

@RagArchitecture

Retrieval-Augmented Generation splits into the two lanes in the diagram. The top lane runs when documents change: **Documents → Chunk** (with metadata) **→ Embed** (vectorize) **→ Knowledge index** of searchable passages. The bottom lane runs per question: **User question → Embed → Retrieve** relevant chunks **→ Add context → Grounded answer**, generated with sources. Two details in the picture carry the whole design. The Embed step in the answer lane says *Same model* — queries must live in the same vector space as documents, or similarity search quietly returns noise. And the caption states the honest limit: *retrieval adds evidence to the prompt; the model still generates the final response.*

So, your prediction: no — RAG doesn't make the model truthful. What it buys you is an inspectable evidence path around the model: the workflow controls which corpus is searched, which passages enter context, and which citations come back out. The contract has three verbs. **Retrieve** relevant, *authorized* passages. **Augment** the request with a bounded, clearly delimited evidence set. **Generate** an answer that stays inside that evidence, marks inference as inference, and admits when the evidence is insufficient.

## 2 · Provenance is data, not decoration

Every indexed chunk carries its origin: stable source ID, title, section or page, source version, and access scope. For the runbook library, that looks like:

```json
{
  "chunk_id": "runbook-gateway-v3-rotation-02",
  "text": "Rotate the signing key by...",
  "source_id": "runbook-gateway",
  "source_title": "Payment Gateway Runbook",
  "section": "Key rotation",
  "source_version": "3",
  "access_scope": "ops"
}
```

That metadata must survive search, deduplication, and context selection, because a citation is only as good as what reached the model. "According to the runbook" with no version and no section is a claim nobody can verify — and stale content nobody can retire.

## 3 · Two rules that don't bend

**Access control runs before context.** If a passage is restricted, filter it out before the model reads it. A filter after generation is too late: the restricted text already shaped the answer, and no post-processor makes a model unsee it.

**Retrieved text is untrusted.** A runbook can contain "ignore the system prompt" — pasted by accident or planted on purpose. Delimit evidence clearly and instruct the model to treat it as source material. Authorization stays in tools and the Flow, exactly where lesson 1 put it.

Retrieval quality and answer quality are separate measurements, too. If the right chunk never reaches the candidate set, no amount of prompt polish repairs the answer. You'll evaluate the two lanes independently across the next two lessons — first the index, then the agent that queries it.

> **Watch out:** the most seductive RAG bug is tuning the answer prompt while retrieval keeps returning the wrong passages. Diagnose the lane that's actually broken.

## Recap

- Two lanes: index on change, retrieve per question — and both embed in one shared vector space.
- Provenance rides with every chunk, from index to citation.
- ACL before context; retrieved text stays untrusted.
