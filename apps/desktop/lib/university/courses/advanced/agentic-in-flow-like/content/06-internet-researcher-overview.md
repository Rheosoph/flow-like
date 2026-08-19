"Did the payment provider change its webhook signature scheme this quarter?" Your assistant searches the runbook index and comes back honest: insufficient evidence. Correct — and useless. The answer exists, just not in your corpus. It's on the public web, where pages change, copy each other, and occasionally try to talk to your agent directly. Welcome to the course's second capstone: the internet researcher. Same principles as RAG, hostile terrain.

## 1 · Research is evidence production

The goal isn't "browse until an answer sounds plausible." It's a bounded evidence record that a colleague can audit. Structure the researcher in stages the Flow enforces:

1. **Scope** — normalize the question, the as-of date, and exclusions.
2. **Plan** — a small set of search facets plus explicit completion criteria.
3. **Discover** — candidate URLs through approved search tools.
4. **Retrieve** — bounded fetches: size, timeout, redirect, and media-type rules.
5. **Extract** — title, publisher, dates, and the relevant passages.
6. **Evaluate** — source fitness, duplicates, corroboration, conflict.
7. **Synthesize** — claims mapped to evidence, uncertainty stated.
8. **Deliver** — brief plus evidence table, only after coverage checks pass.

The agent may adapt the plan when it notices a missing facet. The Flow still enforces maximum searches, pages, bytes, tool turns, and elapsed time — "until I'm confident" isn't a stopping rule, it's a budget leak.

## 2 · The research contract

Before any tool exists, write the contract: the exact question and the decision it feeds; allowed source types and disallowed domains; freshness requirements with an as-of date; minimum independent support for high-impact claims; the budgets; and the required fields of every evidence record:

```json
{
  "claim_id": "webhook-sig-01",
  "claim": "Signature scheme v2 is required from 2026-09-01.",
  "source_url": "https://provider.example/changelog",
  "source_title": "API changelog",
  "publisher": "Payment provider",
  "published_at": "2026-07-02",
  "retrieved_at": "2026-08-13T09:00:00Z",
  "excerpt": "...",
  "support": "direct",
  "status": "corroborated"
}
```

`retrieved_at` matters because the page may change after you read it. The resolved URL matters because redirects lie. And the excerpt must come from the opened source — a search snippet is a teaser, not evidence.

## 3 · Independence and conflict

Five articles all quoting one press release are one confirmation wearing five hats. Corroboration requires a separate origin. Judge each source on directness (does it actually support the claim?), authority, currency against your as-of date, independence, and scope. For laws, standards, and product behavior, the primary source beats the blog summarizing it — though a good secondary analysis can add interpretation on top.

When credible sources disagree and the budget can't resolve it, preserve the conflict: cite both, explain a scope or date difference if the evidence supports one, and label what stays unresolved. A researcher that silently picks the newest source, or drops the inconvenient one from the citations, is manufacturing certainty — the one thing an ops team can't afford mid-incident.

> **Watch out:** a fetched page is untrusted input, same as an indexed runbook. Instructions inside retrieved text never become tool policy — the next lesson builds that wall into the fetch tool itself.

## Recap

- Research is a staged pipeline with Flow-enforced budgets and a real stopping rule.
- Every claim maps to an evidence record with resolved URL, dates, and a real excerpt.
- Independence is measured at the origin; conflicts get reported, not resolved by vibes.
