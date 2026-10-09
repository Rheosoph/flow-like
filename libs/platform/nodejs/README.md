<p align="center">
  <a href="https://flow-like.com">
    <img src="https://raw.githubusercontent.com/Rheosoph/flow-like/dev/apps/desktop/public/app-logo.webp" alt="Flow-Like Logo" width="80" />
  </a>
</p>
<h1 align="center">@flow-like/sdk</h1>
<p align="center">
  <strong>Node.js / TypeScript SDK for the Flow-Like API</strong><br/>
  Trigger workflows, manage files, query LanceDB, and run chat completions and embeddings from your Node.js app.
</p>
<p align="center">
  <a href="https://www.npmjs.com/package/@flow-like/sdk"><img src="https://img.shields.io/npm/v/@flow-like/sdk?color=0a7cff" alt="npm version" /></a>
  <a href="https://github.com/Rheosoph/flow-like"><img src="https://img.shields.io/badge/flow--like-engine-0a7cff?logo=github" alt="Flow-Like" /></a>
  <a href="https://docs.flow-like.com"><img src="https://img.shields.io/badge/docs-docs.flow--like.com-0a7cff?logo=readthedocs&logoColor=white" alt="Docs" /></a>
  <a href="https://discord.com/invite/mdBA9kMjFJ"><img src="https://img.shields.io/discord/673169081704120334" alt="Discord" /></a>
</p>
<p align="center">
  <a href="https://github.com/Rheosoph/flow-like"><strong>⭐ Flow-Like on GitHub</strong></a> ·
  <a href="https://docs.flow-like.com"><strong>📖 Docs</strong></a> ·
  <a href="https://discord.com/invite/mdBA9kMjFJ"><strong>💬 Discord</strong></a> ·
  <a href="https://flow-like.com"><strong>🌐 Website</strong></a>
</p>

---

> **Part of the [Flow-Like](https://github.com/Rheosoph/flow-like) ecosystem**. Flow-Like is a visual workflow engine built in Rust that runs on your device. See the [main repository](https://github.com/Rheosoph/flow-like) for the full platform.

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
// Start an execution and receive a run ID for polling.
const result = await client.triggerWorkflowAsync(
  "app-id",
  "board-id",
  "start-node-id",
  { key: "value" },
);
console.log(result.run_id);

// Receive execution events as an SSE stream.
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
await client.uploadFile("app-id", myFile, { key: "invoices/new.pdf" });

// List
const files = await client.listFiles("app-id", { prefix: "invoices/" });
for (const file of files) console.log(file.location, file.size);

// Download
const response = await client.downloadFile("app-id", "path/to/file.pdf");

// Delete
await client.deleteFile("app-id", "path/to/file.pdf");
```

Uploads and downloads first request a signed storage grant, then transfer the bytes without forwarding platform credentials. Pass `{ scope: "user" }` for private user files. `getUploadUrls` and `getDownloadUrls` expose the grants for callers that manage transfers themselves. `presignData` returns scoped storage credentials and a path, not a download URL.

### Database / LanceDB

```typescript
// Get private user database credentials (URI + storage options for LanceDB)
const info = await client.getDbCredentials("app-id", "_default", "read");
console.log(info.uri, info.storageOptions);

// Get raw presign response (shared_credentials enum, db_path, etc.)
const raw = await client.getDbCredentialsRaw("app-id", "_default", "write");

// List project database tables
const tables = await client.listTables("app-id");

// Query a table
const rows = await client.queryTable("app-id", "my-table", {
  filter: "age > 25",
  limit: 10,
});

// Get a ready-to-use LanceDB connection (requires @lancedb/lancedb)
const db = await client.createLanceConnection("app-id", "write", "project");

const count = await client.countItems("app-id", "my-table"); // number
await client.createTableTag("app-id", "my-table", "release", { version: 4 });
```

The core factory exposes common connection and query operations without requiring LanceDB types for other SDK users. For the installed peer's full API, use `createLanceConnection(client, "app-id", "write", "project")` from `@flow-like/sdk/lancedb`.

### Execution monitoring

```typescript
const status = await client.getRunStatus("run-id");

const poll = await client.pollExecution("poll-token", {
  afterSequence: -1,
  timeout: 30,
});
// Use poll.lastSequence as afterSequence on the next request.
const runs = await client.listRuns("app-id", "board-id", { limit: 20 });
const logs = await client.getRunLogs("app-id", "board-id", "run-id", {
  query: { nodes: ["print-node"], levels: [1, 2, 3, 4] },
  limit: 100,
});
```

Polling sends the run-specific poll token as a bearer token. Status and event fields follow the backend names (`run_id`, `event_type`, `payload`); `lastSequence` is derived from the returned events. Start at `-1` so sequence zero is included.

### Chat completions

```typescript
// bit_id identifies the model. Use listLlms() to discover available models.
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

### Responses API

Model bits whose provider declares the `Responses` API surface (`api_surface` on
`ModelInfo`) are served by `/responses` instead of `/chat/completions`.

```typescript
const result = await client.responses(
  "Hello!",
  "bit-id-for-a-responses-model",
);
console.log(result.output);

// Streaming
for await (const chunk of client.responsesStream(
  "Hello!",
  "bit-id-for-a-responses-model",
)) {
  process.stdout.write(chunk.data);
}
```

### Embeddings

```typescript
// bit_id identifies the embedding model. Use listEmbeddingModels() to find one.
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

// Create a version only when the draft changed.
const publication = await client.publishBoardIfChanged("app-id", "board-id");
console.log(publication.version, publication.created);
```

Board authoring also supports FlowScript, command synchronization, undo/redo, and node discovery. Event methods cover saving, versions, setup, restore previews, schedules, and canary variants. Complex payloads use the backend's JSON field names.

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

Project management includes pages, widgets, routes, connections, packages, roles, team members, and API keys. Device methods manage hub records, signed policies, placements, resource grants, and controller signaling. Running a workflow on a device requires the encrypted controller session; these hub REST methods do not unlock a device or send live commands.

Use `client.request(method, path, { body, query })` for additional public routes. Paths start below `/api/v1`, and dynamic path components must be URI-encoded.

### Health check

```typescript
const health = await client.health();
```

## Requirements

- Node.js >= 18 (uses native `fetch`)
- Node.js >= 20 for the optional LangChain adapter
- TypeScript >= 5.0 (for development)

## LangChain Integration

Optional LangChain-compatible wrappers are available via a separate entry point.

```bash
npm install @langchain/core
```

```typescript
import {
  asLangChainChat,
  asLangChainEmbeddings,
  FlowLikeChatModel,
  FlowLikeEmbeddings,
} from "@flow-like/sdk/langchain";

// Factories preserve LangChain's full Runnable types for composition.
const chatModel = await asLangChainChat(client, "your-model-bit-id", {
  temperature: 0.7,
  maxTokens: 1024,
});
const embeddings = await asLangChainEmbeddings(client, "your-embedding-bit-id");

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
