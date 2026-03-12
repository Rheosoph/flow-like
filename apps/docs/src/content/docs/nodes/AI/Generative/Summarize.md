---
title: Summarize
description: Summarizes long text using an LLM with configurable strategies and optional Chain of Density post-processing.
---

## Purpose of the Node

The **Summarize** node condenses long text into a concise summary using a language model. It supports five different summarization strategies, each with different trade-offs for speed, coherence, and structure preservation. An optional **Chain of Density** post-processing step can increase the information density of the final summary.

For a detailed comparison of all strategies, tuning advice, and best practices, see the [Summarization Strategies](/topics/document-processing/summarization-strategies) guide.

## Pins

| Pin Name | Pin Description | Pin Type | Value Type |
|:----------:|:-------------:|:------:|:------:|
| Input | Execution trigger | Execution | Normal |
| Model | Provider/model bit for summarization | Struct | Bit |
| Text | The long text to summarize (markdown supported) | String | Normal |
| Strategy | Summarization strategy (see below) | String | Normal |
| Densification | Post-processing strategy (None or ChainOfDensity) | String | Normal |
| Instructions | Optional focus instructions (e.g. "focus on action items") | String | Normal |
| Prior Summary | Optional existing summary to extend | String | Normal |
| Chunk Size | Max characters per chunk (default: 8000) | Integer | Normal |
| Chunk Overlap % | Overlap between chunks, 0-50% (default: 10) | Integer | Normal |
| Track Entities | Extract entities across chunks (default: false) | Boolean | Normal |
| Concurrency | Parallel requests for MapReduce/Hybrid (default: 4) | Integer | Normal |
| Max Iterations | Safety limit on summarization passes (default: 5) | Integer | Normal |
| Density Steps | Chain of Density refinement steps, 1-5 (default: 3) | Integer | Normal |
| Output | Fires when summarization is complete | Execution | Normal |
| Summary | The final summarized text | String | Normal |
| Entities | Named entities found (when Track Entities enabled) | String | Array |
| LLM Calls | Total LLM invocations used | Integer | Normal |

## Strategy Overview

| Strategy | Parallelism | Coherence | Best For |
|----------|:-----------:|:---------:|----------|
| **Refine** (default) | None | ★★★★★ | Narratives, meeting notes |
| **MapReduce** | Full | ★★★ | Speed-critical, large docs |
| **Hierarchical** | Partial | ★★★★ | Reports with headings |
| **Hybrid** | Map phase | ★★★★ | Balance of speed & quality |
| **SlidingWindow** | None | ★★★★ | Very long docs (100+ pages) |
