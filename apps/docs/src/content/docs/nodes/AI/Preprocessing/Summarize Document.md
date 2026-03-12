---
title: Summarize Document
description: Creates an intelligent summary of document pages using AI with configurable strategies, detail levels, and optional Chain of Density post-processing.
---

## Purpose of the Node

The **Summarize Document** node takes extracted document pages (from the Extract Document node) and produces a structured summary with keywords and page references. It supports five summarization strategies an three detail levels, allowing fine-grained control over output quality and speed.

Unlike the general-purpose **Summarize** node, this node is specifically designed for document workflows — it preserves page number metadata, extracts keywords, and can generate a table of contents with page references.

## Pins

| Pin Name | Pin Description | Pin Type | Value Type |
|:----------:|:-------------:|:------:|:------:|
| Input | Execution trigger | Execution | Normal |
| Pages | Document pages to summarize | Struct | DocumentPage[] |
| Model | AI model for summarization | Struct | Bit |
| Detail Level | Low, Medium, or High detail | String | Normal |
| Include TOC | Include table of contents with page refs | Boolean | Normal |
| Strategy | Summarization strategy (Refine, MapReduce, etc.) | String | Normal |
| Densification | Post-processing (None or ChainOfDensity) | String | Normal |
| Max Context Tokens | Max characters per chunk (default: 8000) | Integer | Normal |
| Chunk Overlap % | Overlap between chunks, 0-50% (default: 10) | Integer | Normal |
| Track Entities | Extract entities across chunks (default: false) | Boolean | Normal |
| Parallel Requests | Concurrency for MapReduce/Hybrid (default: 4) | Integer | Normal |
| Density Steps | Chain of Density steps, 1-5 (default: 3) | Integer | Normal |
| Output | Fires when summarization completes | Execution | Normal |
| Summary | Structured DocumentSummary with keywords and page refs | Struct | DocumentSummary |

## Detail Levels

| Level | Compression Ratio | Description |
|-------|:--:|-------------|
| **Low** | ~5% | Very concise. Main thesis, key conclusions, critical takeaways only. |
| **Medium** | ~15% | Balanced. Main topics, key arguments, important details and examples. |
| **High** | ~30% | Comprehensive. Most information preserved including evidence and nuances. |

## Output Structure

The `DocumentSummary` output contains:

- **summary**: The generated summary text (markdown formatted)
- **keywords**: Extracted keywords characterizing the document's focus areas. When entity tracking is enabled, tracked entities are merged into this list.
- **page_references**: Topic-to-page mappings showing where each topic appears in the original document.

## Strategies

This node supports the same five strategies as the Summarize node. See the [Summarization Strategies](/topics/document-processing/summarization-strategies) guide for detailed pros/cons, tuning advice, and model recommendations.

| Strategy | Speed | Coherence | Best For |
|----------|-------|-----------|----------|
| **Refine** | Slow | Excellent | Narrative documents, meeting minutes |
| **MapReduce** | Fast | Moderate | Large docs, speed-critical |
| **Hierarchical** | Moderate | Good | Structured reports, papers |
| **Hybrid** | Mixed | Good | Balance of speed and quality |
| **SlidingWindow** | Moderate | Recent-biased | Very long documents (100+ pages) |

## Recommended Configurations

**Quick overview of a long report:**
- Detail Level: Low
- Strategy: MapReduce
- Densification: None

**Thorough summary preserving structure:**
- Detail Level: High
- Strategy: Hierarchical
- Include TOC: true
- Track Entities: true

**Optimal density for sharing:**
- Detail Level: Medium
- Strategy: Refine
- Densification: ChainOfDensity
- Density Steps: 3
