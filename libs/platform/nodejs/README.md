<p align="center">
  <a href="https://flow-like.com">
    <img src="https://raw.githubusercontent.com/TM9657/flow-like/dev/apps/desktop/public/app-logo.webp" alt="Flow-Like Logo" width="80" />
  </a>
</p>
<h1 align="center">@flow-like/sdk</h1>
<p align="center">
  <strong>Node.js / TypeScript SDK for the Flow-Like API</strong><br/>
  Trigger workflows, manage files, query LanceDB, run chat completions & embeddings — all from your Node.js app.
</p>
<p align="center">
  <a href="https://www.npmjs.com/package/@flow-like/sdk"><img src="https://img.shields.io/npm/v/@flow-like/sdk?color=0a7cff" alt="npm version" /></a>
  <a href="https://github.com/TM9657/flow-like"><img src="https://img.shields.io/badge/flow--like-engine-0a7cff?logo=github" alt="Flow-Like" /></a>
  <a href="https://docs.flow-like.com"><img src="https://img.shields.io/badge/docs-docs.flow--like.com-0a7cff?logo=readthedocs&logoColor=white" alt="Docs" /></a>
  <a href="https://discord.com/invite/mdBA9kMjFJ"><img src="https://img.shields.io/discord/673169081704120334" alt="Discord" /></a>
</p>
<p align="center">
  <a href="https://github.com/TM9657/flow-like"><strong>⭐ Flow-Like on GitHub</strong></a> ·
  <a href="https://docs.flow-like.com"><strong>📖 Docs</strong></a> ·
  <a href="https://discord.com/invite/mdBA9kMjFJ"><strong>💬 Discord</strong></a> ·
  <a href="https://flow-like.com"><strong>🌐 Website</strong></a>
</p>

---

> **Part of the [Flow-Like](https://github.com/TM9657/flow-like) ecosystem** — a Rust-powered visual workflow engine that runs on your device. See the [main repository](https://github.com/TM9657/flow-like) for the full platform.

---

## Installation

```bash
npm install @flow-like/sdk
```

For LanceDB integration, also install:

```bash
npm install @lancedb/lancedb
```

## Authentication

The SDK supports two authentication methods:

- **PAT tokens** (prefix `pat_`) → sent as `Authorization: pat_{id}.{secret}`
- **API Keys** (prefix `flk_`) → sent as `X-API-Key: flk_{app_id}.{key_id}.{secret}`

### Via environment variables

```bash
export FLOW_LIKE_BASE_URL=https://api.flow-like.com
export FLOW_LIKE_PAT=pat_myid.mysecret
# or
export FLOW_LIKE_API_KEY=flk_appid.keyid.secret
```

### Via code

```typescript
import { FlowLikeClient } from "@flow-like/sdk";

const client = new FlowLikeClient({
  baseUrl: "https://api.flow-like.com",
  pat: "pat_myid.mysecret",
});
```

## Usage

### Trigger a workflow

```typescript
// Async invoke — returns run ID for polling
const result = await client.triggerWorkflowAsync(
  "app-id",
  "board-id",
  "start-node-id",
  { key: "value" },
);
console.log(result.run_id);

// Sync invoke — returns SSE stream
for await (const event of client.triggerWorkflow(
  "app-id",
  "board-id",
  "start-node-id",
  { key: "value" },
)) {
  console.log(event.data);
}
```

### Trigger an event

```typescript
const result = await client.triggerEventAsync("app-id", "event-id", {
  key: "value",
});
```

### File management

```typescript
// Upload
await client.uploadFile("app-id", myFile);

// List
const files = await client.listFiles("app-id", { prefix: "uploads/" });

// Download
const response = await client.downloadFile("app-id", "path/to/file.pdf");

// Delete
await client.deleteFile("app-id", "path/to/file.pdf");
```

### Database / LanceDB

```typescript
// Get resolved credentials (uri + storageOptions ready for LanceDB)
const info = await client.getDbCredentials("app-id", "_default", "read");
console.log(info.uri, info.storageOptions);

// Get raw presign response (shared_credentials enum, db_path, etc.)
const raw = await client.getDbCredentialsRaw("app-id", "_default", "write");

// List tables
const tables = await client.listTables("app-id");

// Query a table
const rows = await client.queryTable("app-id", "my-table", {
  filter: "age > 25",
  limit: 10,
});

// Get a ready-to-use LanceDB connection (requires @lancedb/lancedb)
const db = await client.createLanceConnection("app-id", "write");
```

### Execution monitoring

```typescript
const status = await client.getRunStatus("run-id");

const poll = await client.pollExecution("poll-token", {
  afterSequence: 0,
  timeout: 30,
});
```

### Chat completions

```typescript
// bit_id identifies the model — use listLlms() to discover available ones
const result = await client.chatCompletions(
  [{ role: "user", content: "Hello!" }],
  "bit-id-for-gpt4",
  { temperature: 0.7 },
);
console.log(result);

// Streaming
for await (const chunk of client.chatCompletionsStream(
  [{ role: "user", content: "Hello!" }],
  "bit-id-for-gpt4",
)) {
  process.stdout.write(chunk.data);
}

// Usage tracking
const usage = await client.getUsage();
console.log(usage.llm_price, usage.embedding_price);
```

### Embeddings

```typescript
// bit_id identifies the embedding model — use listEmbeddingModels() to discover
const result = await client.embed("bit-id-for-embedding", [
  "Hello world",
  "Goodbye world",
]);
console.log(result.embeddings);
```

### Models / Bits

```typescript
// List available LLMs (remote only)
const llms = await client.listLlms();
for (const m of llms) {
  console.log(m.bit_id, m.name, m.provider_name);
}

// List embedding models (remote only)
const embeddings = await client.listEmbeddingModels();

// Search all bits
const bits = await client.searchBits({ search: "llama", bit_types: ["Llm"] });

// Get a specific bit
const bit = await client.getBit("some-bit-id");
```

### Board management

```typescript
// List boards
const boards = await client.listBoards("app-id");

// Read a board
const board = await client.getBoard("app-id", "board-id");

// Create / update a board
const { id } = await client.upsertBoard("app-id", "board-id", {
  name: "My Board",
  description: "Does things",
});

// Delete a board
await client.deleteBoard("app-id", "board-id");

// Pre-run analysis
const prerun = await client.prerunBoard("app-id", "board-id");
console.log(prerun.runtime_variables);
```

### HTTP Sink

```typescript
const result = await client.triggerHttpSink("app-id", "webhook/path", "POST", {
  event: "user.created",
});
```

### App management

```typescript
const apps = await client.listApps();
const app = await client.getApp("app-id");
const newApp = await client.createApp("My App", "Description");
```

### Health check

```typescript
const health = await client.health();
```

## Requirements

- Node.js >= 18 (uses native `fetch`)
- TypeScript >= 5.0 (for development)

## LangChain Integration

Optional LangChain-compatible wrappers are available via a separate entry point.

```bash
npm install @langchain/core
```

```typescript
import { FlowLikeChatModel, FlowLikeEmbeddings } from "@flow-like/sdk/langchain";

// Option 1: Factory methods from an existing client (recommended)
const chatModel = await client.asLangChainChat("your-model-bit-id", {
  temperature: 0.7,
  maxTokens: 1024,
});
const embeddings = await client.asLangChainEmbeddings("your-embedding-bit-id");

// Option 2: Standalone (requires baseUrl + token)
const chatModel2 = new FlowLikeChatModel({
  baseUrl: "https://api.flow-like.com",
  token: "pat_myid.mysecret",
  bitId: "your-model-bit-id",
  temperature: 0.7,
});

// Use with LangChain
const response = await chatModel.invoke("Hello, how are you?");

const embeddings2 = new FlowLikeEmbeddings({
  baseUrl: "https://api.flow-like.com",
  token: "pat_myid.mysecret",
  bitId: "your-embedding-bit-id",
});

const vectors = await embeddings.embedDocuments(["Hello world", "Goodbye world"]);
const queryVector = await embeddings.embedQuery("search query");
```
