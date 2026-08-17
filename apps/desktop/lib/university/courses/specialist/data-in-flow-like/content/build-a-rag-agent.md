A partner team heard about your copilot and built their own for their market: **PolicyPal**, a support-policy assistant over the same kind of corpus — a refund policy, a support playbook, and a pricing addendum restricted to enterprise accounts. It launches in a week. Their lead has asked you for a design review: "It mostly works. QA found some oddities."

They sent two artifacts: the as-built notes and QA's symptom log. Read both with the whole course in mind — data boundaries, file identity, idempotent writes, query grain, RAG invariants, ontology rules. Every question that follows asks you to connect a symptom or a design choice to the invariant it violates. None of the oddities are accidents; each one traces back to a specific decision below.

## The as-built notes

1. All policy documents live in one folder of project Storage; the enterprise pricing addendum sits alongside the public policies.
2. One workflow does everything: on each incoming question it re-extracts and re-embeds every document, then retrieves and answers.
3. Chunk rows get their IDs from the source filename plus the indexing run's timestamp and a counter — "guaranteed unique."
4. Chunks are written with Insert; the write node's error pin routes to a log-and-continue branch so failures never stop a run.
5. Right after writing, the same workflow counts the chunk table and, when the number looks plausible, reports "index ready."
6. Retrieval is full-text only — "policy codes like REF-201 are exact identifiers, and exact search is predictable."
7. Access control: after the model answers, a post-processing step strips out the names of any documents the caller isn't entitled to see.
8. For graph exploration, chunk rows are copied nightly into a separate `graph_chunks` table, "so the ontology doesn't touch production data."
9. QA sign-off: ten sample questions, each rated by a reviewer on how fluent and confident the answer reads.

## The symptom log

- **S1** — Every answer takes about 90 seconds, and both latency and cost grow every time a document is added to the corpus.
- **S2** — After a failed indexing run was retried, several passages now appear twice in answers. Separately, one "index ready" report went out although the count behind it later turned out to be the *previous* run's total, and a question asked seconds after the report found none of the new content.
- **S3** — "What is REF-201?" answers perfectly. "Can I get money back on an annual plan?" returns nothing useful, even though the refund policy covers it.
- **S4** — A trial-tier caller asked about discounts and quoted figures that exist only in the enterprise pricing addendum. The addendum's title appeared nowhere in the answer.
- **S5** — The refund policy was updated to v2, yet old refund windows still surface in answers — even after the team deleted rows from `graph_chunks`.

@RAGOverview

Keep the two-lane diagram in view while you work: an index lane that runs when sources change, an answer lane that runs per question, and a single knowledge index carrying evidence between them. Every symptom above is a departure from that picture or from the write discipline you built in lessons 2 through 4. The challenges below are the review meeting — name what broke, and be ready for the moment the lead asks, "so what should it do instead?"
