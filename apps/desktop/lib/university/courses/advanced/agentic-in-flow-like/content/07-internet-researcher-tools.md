Mid-run, your researcher decides its next fetch should be `http://169.254.169.254/latest/meta-data/`. That's not a website — it's the cloud metadata endpoint, where credentials live. No page told it to; the model simply generated a URL, because URLs are just strings.

> **Predict first:** which line of which tool contract stops this request? By the end of this lesson, you'll have written it.

## 1 · Four small tools, not one browser

Give the researcher four narrow read tools instead of a generic browser:

```text
search_sources(query, domains?, recency_days?, limit <= 10)
  -> {results: [{url, title, snippet}], truncated}

fetch_source(url, max_bytes, timeout_ms)
  -> {resolved_url, status, media_type, retrieved_at, body_ref}

extract_evidence(body_ref, question, max_passages)
  -> {source, passages: [{excerpt, location, relevance}]}

store_research_evidence(run_id, records)
  -> {accepted_count, rejected: [{index, reason}]}
```

Each boundary is separately authorized, limited, and observable. `search_sources` clamps result counts and uses an approved provider. `extract_evidence` returns bounded passages, not whole pages. `store_research_evidence` validates the evidence schema and scopes to the current run. Notice what's absent: no write, no publish. Drafting and sending are different tools with different confirmation rules — this release registers neither.

## 2 · fetch_source is the wall

The answer to the opening question lives in `fetch_source`. A model-provided URL is untrusted input, so the implementation allows only intended schemes and hosts, blocks internal and metadata network destinations, resolves redirects and rechecks the policy on every hop, enforces `max_bytes` and a media-type allowlist before reading the body, and validates status before anything is parsed. The metadata request dies at the host policy — before a single byte moves. Strip credentials and sensitive query parameters from anything that reaches logs or citations.

Build the HTTP path visibly with Flow-Like's HTTP nodes: create the request, set URL and method, add headers from secret-backed values (never a board constant), invoke **API Call**, validate status, then parse. Retry only transient failures — network blips, rate limits, appropriately classified server errors — with backoff, inside the total research budget. A `404` retried five times is still a `404`.

## 3 · From pages to claims

Chain the tools: search → fetch → extract → dedupe → synthesize. Deduplication compares canonical URLs *and* content similarity, because syndicated articles share text, not addresses. Build a claim-evidence table where every substantive claim references at least one stored record. Register the read tools with **Register Function Tools**, install lesson 6's contract as the system prompt, invoke under maximum turns and elapsed time, and return the brief and the evidence table as separate fields.

Then test like an adversary: no results, a stale page, a redirect that leaves policy mid-chain, an oversized body, an unsupported media type, a rate limit, a timeout, syndicated duplicates, conflicting sources, a page containing instructions addressed to the agent — and one model-proposed out-of-scope query, to confirm the workflow rejects it. A polished brief with an unsupported claim is a failing run.

> **Watch out:** a timeout is an unknown result, not a proven failure — don't count it as "no effect", and don't retry it blindly outside the budget.

## Recap

- Four narrow tools — search, fetch, extract, store — each with its own limits.
- fetch_source enforces scheme, host, redirect, size, and type policy before content exists.
- Every claim in the brief maps to a stored evidence record.
