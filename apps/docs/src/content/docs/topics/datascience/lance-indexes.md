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

An FM-index is a compressed index for finding an exact substring in raw strings or bytes. It does not tokenize text for relevance ranking.

![A Lance index decision tree for vectors, text, scalar values, lists, and lower-level pruning. The following table provides the full text equivalent.](../../../../assets/LanceIndexDecisionGuide.svg)

## Choose from the query

The table is the text equivalent of the decision tree and includes the less common upstream choices.

| Repeated query | Start with | Availability in Flow-Like |
|----------------|------------|---------------------------|
| None, or the current scan is already fast | No index | Available |
| Exact nearest vectors on a manageable table | Flat vector scan | Available |
| Approximate nearest vectors at scale | A vector index chosen by recall, latency, memory, and storage tests | Cosine IVF-PQ is available; other algorithms and tuning controls **require an update** |
| Words, phrases, or BM25-ranked text | `FULL TEXT` | Available |
| Arbitrary substring filters through `contains` on string or binary data | `FM` | **Requires an update** |
| Repeated `contains` or `LIKE` filters with a usable three-character literal | `NGRAM` | Lower-level Lance API only |
| Point, range, `IN`, or null filters on mostly distinct scalar values | `BTREE` | Available |
| Point, range, `IN`, or null filters on a few distinct scalar values | `BITMAP`; fewer than about 1,000 unique values is the upstream starting heuristic | Available |
| Any or all membership tests inside a `List<T>` or `LargeList<T>` of primitive, low-cardinality values | `LABEL LIST` | Available |
| Skip zones using coarse minimum and maximum bounds | `ZONEMAP` | Lower-level Lance API only |
| Skip zones using approximate membership with possible false positives | `BLOOMFILTER` | Lower-level Lance API only |
| Prune two-dimensional bounding-box searches | `RTREE` | Lower-level Lance API only |

`NGRAM`, `ZONEMAP`, `BLOOMFILTER`, and `RTREE` already exist in Flow-Like's pinned Lance 4 crates and remain in Lance 10.0.0. LanceDB Rust 0.37.1 does not expose them through its regular `create_index` builder, so a dependency update alone will not add those choices. Flow-Like needs lower-level integration for them.

## What Build Index does today

The [Build Index](/nodes/data/database/optimization/index-local-db/) workflow node flushes buffered writes, then builds one index on one column. It exposes the builders below without algorithm, distance-metric, partition, quantization, or text-tokenizer settings. LanceDB 0.37.1 still does not support a composite index across several columns.

| Selection | Current behavior |
|-----------|------------------|
| `BTREE` | Builds a B-tree index |
| `BITMAP` | Builds a bitmap index |
| `LABEL LIST` | Builds a label-list index |
| `FULL TEXT` | Builds a full-text inverted index |
| `VECTOR` | Builds an IVF-PQ index with cosine distance |
| `AUTO` | Builds the same cosine IVF-PQ index for a supported vector column; otherwise delegates to LanceDB `Auto`, which chooses B-tree for a supported scalar column |

`AUTO` does not inspect scalar cardinality. Choose `BITMAP` or `LABEL LIST` yourself when the query shape calls for one. On a vector column, `VECTOR` and `AUTO` take the same cosine IVF-PQ path.

The HTTP and Data Studio builder omits the `VECTOR` selection. Its automatic vector route is `AUTO`.

:::caution[Rebuild vector indexes created before this fix]
Flow-Like's Vector Search and Hybrid Search request cosine distance, and new `VECTOR` and vector `AUTO` indexes now use cosine as well. An existing index keeps the metric it was trained with. Run Build Index again on each affected vector column to replace a legacy L2 index, then compare recall with the flat-search baseline.
:::

## Choose a vector index after the update

An approximate nearest-neighbor (ANN) index trades some recall for lower latency. Current stable LanceDB combines inverted-file (IVF) partitions with full vectors, scalar quantization (SQ), product quantization (PQ), RaBitQ quantization (RQ), or a hierarchical navigable small-world (HNSW) graph.

| Workload goal | Starting candidate | Practical consequence |
|---------------|--------------------|-----------------------|
| Exact results, or a table small enough to scan | No vector index | Reads every eligible vector and provides the comparison baseline |
| Highest recall without vector quantization | `IVF_HNSW_FLAT` | Keeps full vectors and uses more memory and storage |
| Strong recall and latency with lower memory use | `IVF_HNSW_SQ` | Quantizes each vector value and usually gives the best general starting point |
| Maximum compression or a filter-heavy workload | `IVF_RQ` | Compresses aggressively; verify recall on representative queries |
| Vectors with at most 256 dimensions, especially with filters | `IVF_PQ` | Compresses subvectors; tune partitions, probes, and refinement |

LanceDB 0.37.1 also exposes `IVF_FLAT`, `IVF_SQ`, and `IVF_HNSW_PQ`. An IVF index still searches selected partitions, so `IVF_FLAT` is approximate unless the query probes every partition. HNSW variants can show more latency variation under heavy filtering. Benchmark at least two plausible candidates.

An incremental update adds appended rows using the existing IVF partitions and, when applicable, the existing quantization model. It does not retrain IVF centroids or codebooks. Rebuild a vector index after major growth or a distribution shift, then measure recall again.

Build and query with the same distance metric. Record recall against the flat baseline, p50 and p95 latency, index size, build time, and write cost. The fastest result is useful only if its recall meets the product requirement.

## Additional scalar indexes

FM is new relative to Flow-Like's pinned high-level LanceDB stack. The other indexes in this section need lower-level plumbing even though their implementations already exist in the pinned Lance crates.

| Index | Use it for | Boundary |
|-------|------------|----------|
| `FM` | Exact raw substring filters through `contains` on string or binary data | LanceDB 0.37.1 exposes it; Flow-Like does not yet |
| `NGRAM` | Repeated text `contains` and `LIKE` predicates | ASCII-folded, lower-case trigrams; short patterns fall back to row checks |
| `ZONEMAP` | Cheap zone pruning when values cluster into useful min/max ranges | Lower-level Lance API only |
| `BLOOMFILTER` | Cheap zone pruning for equality or membership tests | May return false positives, which the query verifies against rows |
| `RTREE` | Static two-dimensional bounding-box pruning for GeoArrow geometry | Lower-level Lance API only |

The high-level LanceDB FM builder documents `contains` as its supported predicate. The lower-level Lance implementation can plan additional prefix, suffix, and regex operations. FM works on raw bytes and remaps `0x00` and `0xFF` to spaces, so it is unsuitable when those byte values must remain distinct.

Full-text search has a different purpose from FM and N-gram indexes. It tokenizes documents and supports term, phrase, and BM25-ranked retrieval. Use it when the meaning of a match is based on tokens rather than a raw substring.

## Build and maintain the index

:::caution[Version cleanup is explicit]
**Optimize and Update** now defaults **Keep Versions?** to true. In that mode it compacts fragments and updates indexes without pruning version history.

Disabling **Keep Versions?** runs the same maintenance first, then prunes versions older than seven days. Cleanup leaves unverified files untouched, which protects files that may belong to another process's in-progress operation. A tag that references an old version blocks pruning, and a pruned version cannot be checked out or restored. Flow-Like does not currently expose a different retention period.
:::

1. Capture the real filter or search and measure its unindexed latency.
2. Inspect the column type, cardinality, selectivity, and update pattern.
3. Pick the narrowest supported index from the decision table. Build it after the bulk load.
4. Use [List Indices](/nodes/data/database/meta/list-indices-db/) to record the generated index name, type, and column.
5. After substantial appends, run [Optimize and Update](/nodes/data/database/optimization/optimize-local-db/) with **Keep Versions?** enabled. Disable it only when the seven-day cleanup policy fits the database's retention requirements.
6. Repeat the same workload. Keep the index only when the latency gain justifies its build, write, and storage costs.

LanceDB queries scan rows that are newer than the indexed data and merge them with indexed results. The results remain complete, while latency can rise as uncovered rows accumulate. Upstream APIs expose index statistics for checking coverage. Flow-Like currently bundles index maintenance into **Optimize and Update**.

Use [Drop Index](/nodes/data/database/optimization/drop-index-db/) with the name returned by List Indices when an index is ineffective, obsolete, or built with the wrong vector metric.

## Upstream references

- [LanceDB indexing guide](https://docs.lancedb.com/indexing)
- [LanceDB scalar index choices](https://docs.lancedb.com/indexing/scalar-index)
- [LanceDB vector index choices](https://docs.lancedb.com/indexing/vector-index)
- [LanceDB reindexing and index coverage](https://docs.lancedb.com/indexing/reindexing)
- [Current Lance index-format specification](https://lance.org/format/index/)
- [Scalar index types in the tagged Lance 10.0.0 source](https://github.com/lance-format/lance/blob/v10.0.0/rust/lance-index-core/src/scalar.rs#L39-L70)
- [Lance 10.0.0 release](https://github.com/lance-format/lance/releases/tag/v10.0.0)
- [LanceDB 0.37.1 release](https://github.com/lancedb/lancedb/releases/tag/v0.37.1)
