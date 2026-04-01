---
title: SDK Overview
description: Official client SDKs for integrating with the Flow-Like API from Node.js/TypeScript and Python.
sidebar:
  order: 0
---

Flow-Like provides official SDKs for **Node.js/TypeScript** and **Python** that wrap the REST API into ergonomic, type-safe clients. Use them to control workflows, manage files, query LanceDB databases, run chat completions, generate embeddings, and more — all from your application code.

## Available SDKs

| Language | Package | Install |
|----------|---------|---------|
| Node.js / TypeScript | [`@flow-like/sdk`](https://www.npmjs.com/package/@flow-like/sdk) | `npm install @flow-like/sdk` |
| Python | [`flow-like`](https://pypi.org/project/flow-like/) | `uv add flow-like` |

## Feature Matrix

| Feature | Node.js | Python |
|---------|---------|--------|
| Trigger workflows (sync SSE + async) | ✅ | ✅ |
| Trigger events | ✅ | ✅ |
| File management (upload, download, list, delete) | ✅ | ✅ |
| LanceDB integration (credentials, queries, connections) | ✅ | ✅ |
| Chat completions (+ streaming) | ✅ | ✅ |
| Embeddings | ✅ | ✅ |
| Model discovery (LLMs, embeddings) | ✅ | ✅ |
| Board CRUD | ✅ | ✅ |
| App management | ✅ | ✅ |
| HTTP sinks | ✅ | ✅ |
| Execution monitoring / polling | ✅ | ✅ |
| LangChain integration | ✅ | ✅ |
| Async / await | ✅ native | ✅ `a`-prefixed methods |

## Authentication

Both SDKs support two authentication methods:

| Type | Prefix | Header sent | Env variable |
|------|--------|-------------|-------------|
| Personal Access Token | `pat_` | `Authorization: pat_{id}.{secret}` | `FLOW_LIKE_PAT` |
| API Key | `flk_` | `X-API-Key: flk_{app}.{key}.{secret}` | `FLOW_LIKE_API_KEY` |

The SDK auto-detects which header to use based on the token prefix.

### Environment variables

Both SDKs read from environment variables by default:

```bash
export FLOW_LIKE_BASE_URL=https://api.flow-like.com
export FLOW_LIKE_PAT=pat_myid.mysecret
# or
export FLOW_LIKE_API_KEY=flk_appid.keyid.secret
```

## LangChain Integration

Both SDKs include optional [LangChain](https://www.langchain.com/)-compatible wrappers so you can use Flow-Like models inside LangChain chains, agents, and RAG pipelines.

- **Node.js**: `import { FlowLikeChatModel, FlowLikeEmbeddings } from "@flow-like/sdk/langchain"`
- **Python**: `from flow_like.langchain import FlowLikeChatModel, FlowLikeEmbeddings`

See the language-specific pages for installation and usage details.

## Next Steps

- [Node.js / TypeScript SDK →](../nodejs)
- [Python SDK →](../python)
