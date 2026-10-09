---
title: SDK Overview
description: Call the Flow-Like platform API from TypeScript, Python, and Rust.
sidebar:
  order: 0
---

Use the platform SDKs to publish workflow versions, invoke Events, inspect runs,
and manage project data from application code. TypeScript, Python, and Rust
clients call the same REST API and use the caller's existing permissions.

This documentation describes the SDK source in this repository. Newly added
methods require this revision until a package release includes them. The Rust
client is currently available as a source dependency.

## Available SDKs

| Language | Package | Install |
|----------|---------|---------|
| Node.js / TypeScript | [`@flow-like/sdk`](https://www.npmjs.com/package/@flow-like/sdk) | `npm install @flow-like/sdk` |
| Python | [`flow-like`](https://pypi.org/project/flow-like/) | `uv add flow-like` |
| Rust | `flow-like-platform` | [Use the source crate](/dev/sdks/rust/) |

## Feature Matrix

The clients cover workflow invocation, publication and editing, Event management,
run history and logs, signed file transfers, database administration, app access,
frontend resources, packages, connections, and device hub management. Complex
management payloads retain the backend's JSON field names. TypeScript and Python
also include optional LanceDB connection and LangChain adapters; the Rust client
provides database credentials and REST queries without those adapters.

Python exposes synchronous methods and `a`-prefixed async methods. TypeScript
uses promises and async iterables. Rust uses async methods and an event stream.

Device hub methods manage inventory, grants, and deployments. They do not open
the encrypted agent channel. Starting a device service or reading its local
execution logs still uses [device management](/devices/service-access/); Rust applications
can use the separate `flow-like-device-client` crate for that channel.

## Upgrading from 0.1

The TypeScript and Python source versions are prepared for a `0.2.0` release.
Check these corrected contracts when upgrading:

| API | Caller change |
| --- | --- |
| TypeScript `listFiles()` | Read the returned array directly. File entries use `location`, `last_modified`, and `is_dir`. |
| TypeScript `countItems()` | Use the returned number directly. |
| TypeScript run status and poll events | Use the backend names, including `run_id`, `event_type`, and `payload`. |
| TypeScript optional integrations | Import factories from `@flow-like/sdk/langchain` or `@flow-like/sdk/lancedb` when you need the peer library's full types. Core factories expose common operations. |
| File uploads | Supply a destination key for bytes or an unnamed Blob. |
| `presignData()` / `presign_data()` | Read scoped storage credentials and a path. Use the upload/download URL helpers for individual files. |
| Polling | Start at sequence `-1` to include the first event. Advance the cursor through all remaining pages, including after completion. |

Python retains its `FileInfo` and `CountResult` wrappers. Complex management
documents in all three clients follow the backend's JSON field names.

## Choose a model API

Model discovery returns `api_surface` alongside `bit_id`. Use that field when
choosing the client method:

| `api_surface` | Node.js | Python | Request shape |
| --- | --- | --- | --- |
| `ChatCompletions` or absent | `chatCompletions()` | `chat_completions()` | `messages`, `max_tokens` |
| `Responses` | `responses()` | `responses()` | `input`, `max_output_tokens` |

These are separate request and response contracts. The server rejects a model
sent to the wrong endpoint with HTTP 400 and names the required route. See the
[Node.js examples](/dev/sdks/nodejs/#chat-completions) and
[Python examples](/dev/sdks/python/#chat-completions) for discovery and calls.

## Authentication

The platform SDKs support these authentication methods:

| Type | Prefix | Header sent | Env variable |
|------|--------|-------------|-------------|
| Personal Access Token | `pat_` | `Authorization: pat_{id}.{secret}` | `FLOW_LIKE_PAT` |
| API Key | `flk_` | `X-API-Key: flk_{app}.{key}.{secret}` | `FLOW_LIKE_API_KEY` |

The SDK sends PATs in `Authorization` and App API keys in `X-API-Key`.
Use the explicit `pat` / `apiKey` options in Node.js, or `pat` / `api_key` in
Python. Python also accepts a `token` option and detects its prefix. Provide
one credential type at a time. Rust accepts an explicit `Auth` value or reads
the same environment variables through `Client::from_env()`.

Async invocations return a separate `poll_token`. Polling uses that token as
Bearer authorization. Signed file transfers use the storage URL's authorization;
neither operation forwards the client's platform credential.

### Environment variables

Use these environment variables for client configuration:

```bash
export FLOW_LIKE_BASE_URL=https://api.flow-like.com
export FLOW_LIKE_PAT=pat_myid.mysecret
# or
export FLOW_LIKE_API_KEY=flk_appid.keyid.secret
```

## LangChain Integration

The TypeScript and Python SDKs include optional [LangChain](https://www.langchain.com/)-compatible wrappers so you can use Flow-Like models inside LangChain chains, agents, and RAG pipelines.

- **Node.js**: `import { FlowLikeChatModel, FlowLikeEmbeddings } from "@flow-like/sdk/langchain"`
- **Python**: `from flow_like.langchain import FlowLikeChatModel, FlowLikeEmbeddings`

See the language-specific pages for installation and usage details.

## Next Steps

- [Node.js / TypeScript SDK →](../nodejs)
- [Python SDK →](../python)
- [Rust SDK →](../rust)
