This optional class adds a bounded web lookup to the agent. Prerequisites: a working HTTP flow, an approved search provider, and a fetch service that enforces destination, redirect, response-size and timeout limits. Allow setup time separately.

## Build one evidence path

Choose one current product question that its vendor documentation can answer. Limit the exercise to two searches and three opened pages. Implement two Flow functions: `search_sources(query)` returning candidate URLs, and `fetch_source(url)` returning the resolved URL, publisher, retrieval time and selected source text.

The fetch service must check the resolved destination and every redirect. A URL check before DNS resolution alone does not block internal destinations. If your HTTP setup cannot enforce that boundary, use fixed vendor URLs in the function and expose no URL argument to the model.

Register the two functions. Ask the agent to return a small evidence table with claim, URL, retrieval time and supporting excerpt. Inspect each opened page yourself and compare the claims. Search snippets are discovery data; they are insufficient as source evidence.

## Test a failure

Have the fetch function return `unavailable` for one of the selected URLs. The answer must name the missing evidence and avoid attributing a claim to that page. Also test two pages that quote the same original announcement: they provide one origin of evidence.

Completion: keep one successful table and one degraded result, with the observed calls. Stop on the configured search/page limits. This class adds web evidence; the RAG course owns indexing and retrieval from your documents.
