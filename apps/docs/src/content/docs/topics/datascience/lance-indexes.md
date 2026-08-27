---
title: Choosing a Lance Index
description: Choose and maintain Lance indexes for filters, text search, list membership, and vector retrieval
sidebar:
  order: 3
---

Choose an index from the query you need to accelerate. A useful index avoids reading most rows for a repeated, selective query. A rare query, a broad filter, or a small table may be faster and cheaper to scan.

Flow-Like's local database uses LanceDB over the Lance columnar format. A physical index stores an additional route from a search key to candidate row IDs. It consumes storage, takes time to build, and needs maintenance as new data arrives.

:::caution[Version scope]
This guide was checked on 27 August 2026 against [Lance 10.0.0](https://github.com/lance-format/lance/releases/tag/v10.0.0) and [LanceDB Rust 0.37.1](https://github.com/lancedb/lancedb/releases/tag/v0.37.1), the newest stable releases. The literal newest tags, Lance 12.0.0-beta.3 and LanceDB 0.38.0-beta.11, were previews, so they are excluded from the recommendations. The `10.0.0` number identifies the Lance crate release, not an on-disk file-format version.

Flow-Like still pins Lance 4.0.0 and LanceDB 0.27.2. Options marked **Requires an update** describe the stable upstream capability to target when those dependencies and Flow-Like's index controls are updated.
:::

![A decision tree that starts with the repeated query and routes similar vectors to a flat scan or tuned vector index, token search to Full Text, raw substring search to FM, scalar filters to BTree or Bitmap, list membership to Label List, and coarse pruning or geometry to upstream Lance 10 indexes](../../../../assets/LanceIndexDecisionGuide.svg)

## Choose from the query

The table is the text equivalent of the decision tree and includes the less common upstream choices.

| Repeated query | Start with | Availability in Flow-Like |
|----------------|------------|---------------------------|
| None, or the current scan is already fast | No index | Available |
| Exact nearest vectors on a manageable table | Flat vector scan | Available |
| Approximate nearest vectors at scale | A vector index chosen by recall, latency, memory, and storage tests | **Requires an update** for an explicit metric and algorithm |
| Words, phrases, or BM25-ranked text | `FULL TEXT` | Available |
| Arbitrary substring, prefix, suffix, regex, or byte search | `FM` | **Requires an update** |
| Repeated `contains` or `LIKE` filters on text | `NGRAM` | Direct Lance 10 only |
| Point, range, `IN`, or null filters on mostly distinct scalar values | `BTREE` | Available |
| Equality or small `IN` filters on a few distinct scalar values | `BITMAP`; fewer than about 1,000 unique values is the upstream starting heuristic | Available |
| Any or all membership tests inside a list column | `LABEL LIST` | Available |
| Skip pages using coarse minimum and maximum bounds | `ZONEMAP` | Direct Lance 10 only |
| Skip pages using approximate membership with possible false positives | `BLOOMFILTER` | Direct Lance 10 only |
| Prune two-dimensional bounding-box searches | `RTREE` | Direct Lance 10 only |

`NGRAM`, `ZONEMAP`, `BLOOMFILTER`, and `RTREE` are available in the Lance 10.0.0 format engine and lower-level API. LanceDB Rust 0.37.1 does not expose them through its regular `create_index` builder, so a Flow-Like dependency update alone will not add those choices.

## What Build Index does today

The [Build Index](/nodes/data/database/optimization/index-local-db/) workflow node flushes buffered writes, then builds one index on one column. It exposes the builders below without algorithm, distance-metric, partition, quantization, or text-tokenizer settings.

| Selection | Current behavior |
|-----------|------------------|
| `BTREE` | Builds a B-tree index |
| `BITMAP` | Builds a bitmap index |
| `LABEL LIST` | Builds a label-list index |
| `FULL TEXT` | Builds a full-text inverted index |
| `VECTOR` | Delegates to LanceDB `Auto` |
| `AUTO` | Chooses IVF-PQ with L2 distance for a fixed-size vector column; otherwise chooses B-tree for a supported scalar column |

`AUTO` does not inspect scalar cardinality. Choose `BITMAP` or `LABEL LIST` yourself when the query shape calls for one. `VECTOR` and `AUTO` currently take the same implementation path.

The HTTP and Data Studio builder omits the `VECTOR` selection. Its automatic vector route is `AUTO`.

:::danger[Do not use the current automatic vector index for cosine search]
Flow-Like's Vector Search and Hybrid Search request cosine distance. LanceDB's current `Auto` builder creates an IVF-PQ vector index with L2 distance, and LanceDB requires the index and query distance metrics to match. This mismatch can produce invalid nearest-neighbor results.

Until Flow-Like exposes a cosine-aware vector builder, keep cosine retrieval on a flat scan or rebuild the index through an integration that sets cosine explicitly. Compare any indexed result with a flat-search baseline before relying on it.
:::

## Choose a vector index after the update

An approximate nearest-neighbor (ANN) index trades some recall for lower latency. Current stable LanceDB combines inverted-file (IVF) partitions with full vectors, scalar quantization (SQ), product quantization (PQ), residual quantization (RQ), or a hierarchical navigable small-world (HNSW) graph.

| Workload goal | Starting candidate | Practical consequence |
|---------------|--------------------|-----------------------|
| Exact results, or a table small enough to scan | No vector index | Reads every eligible vector and provides the comparison baseline |
| Highest recall without vector quantization | `IVF_HNSW_FLAT` | Keeps full vectors and uses more memory and storage |
| Strong recall and latency with lower memory use | `IVF_HNSW_SQ` | Quantizes each vector value and usually gives the best general starting point |
| Maximum compression or a filter-heavy workload | `IVF_RQ` | Compresses aggressively; verify recall on representative queries |
| Vectors with at most 256 dimensions, especially with filters | `IVF_PQ` | Compresses subvectors; tune partitions, probes, and refinement |

LanceDB 0.37.1 also exposes `IVF_FLAT`, `IVF_SQ`, and `IVF_HNSW_PQ`. An IVF index still searches selected partitions, so `IVF_FLAT` is approximate unless the query probes every partition. HNSW variants can show more latency variation under heavy filtering. Benchmark at least two plausible candidates.

Build and query with the same distance metric. Record recall against the flat baseline, p50 and p95 latency, index size, build time, and write cost. The fastest result is useful only if its recall meets the product requirement.

## New scalar choices in Lance 10

The latest stable Lance release widens the set of physical indexes, but each one accelerates a specific predicate.

| Index | Use it for | Boundary |
|-------|------------|----------|
| `FM` | Exact raw substring, prefix, suffix, regex, or byte search | LanceDB 0.37.1 exposes it; Flow-Like does not yet |
| `NGRAM` | Repeated text `contains` and `LIKE` predicates | Lower-level Lance API only |
| `ZONEMAP` | Cheap page pruning when values cluster into useful min/max ranges | Lower-level Lance API only |
| `BLOOMFILTER` | Cheap page pruning for equality or membership tests | May return false positives, which the query must verify |
| `RTREE` | Static two-dimensional bounding-box pruning for GeoArrow geometry | Lower-level Lance API only |

Full-text search has a different purpose from `FM` and `NGRAM`. It tokenizes documents and supports term, phrase, and BM25-ranked retrieval. Use it when the meaning of a match is based on tokens rather than a raw substring.

## Build and maintain the index

1. Capture the real filter or search and measure its unindexed latency.
2. Inspect the column type, cardinality, selectivity, and update pattern.
3. Pick the narrowest supported index from the decision table. Build it after the bulk load.
4. Use [List Indices](/nodes/data/database/meta/list-indices-db/) to record the generated index name, type, and column.
5. After substantial appends, run [Optimize and Update](/nodes/data/database/optimization/optimize-local-db/) to compact fragments and update existing indexes.
6. Repeat the same workload. Keep the index only when the latency gain justifies its build, write, and storage costs.

LanceDB queries scan rows that are newer than the indexed data and merge them with indexed results. The results remain complete, while latency can rise as uncovered rows accumulate. Upstream APIs expose index statistics for checking coverage. Flow-Like's maintenance path is **Optimize and Update**.

Use [Drop Index](/nodes/data/database/optimization/drop-index-db/) with the name returned by List Indices when an index is ineffective, obsolete, or built with the wrong vector metric.

## Upstream references

- [LanceDB indexing guide](https://docs.lancedb.com/indexing)
- [LanceDB vector index choices](https://docs.lancedb.com/indexing/vector-index)
- [LanceDB reindexing and index coverage](https://docs.lancedb.com/indexing/reindexing)
- [Index format and index taxonomy in Lance 10.0.0](https://lance.org/format/index/)
- [Lance 10.0.0 release](https://github.com/lance-format/lance/releases/tag/v10.0.0)
- [LanceDB 0.37.1 release](https://github.com/lancedb/lancedb/releases/tag/v0.37.1)
