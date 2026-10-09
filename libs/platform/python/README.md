<p align="center">
  <a href="https://flow-like.com">
    <img src="https://raw.githubusercontent.com/Rheosoph/flow-like/dev/apps/desktop/public/app-logo.webp" alt="Flow-Like Logo" width="80" />
  </a>
</p>
<h1 align="center">flow-like</h1>
<p align="center">
  <strong>Python SDK for the Flow-Like API</strong><br/>
  Trigger workflows, manage files, query LanceDB, run chat completions, embeddings, and device management from Python.
</p>
<p align="center">
  <a href="https://pypi.org/project/flow-like/"><img src="https://img.shields.io/pypi/v/flow-like?color=0a7cff" alt="PyPI version" /></a>
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

> **Part of the [Flow-Like](https://github.com/Rheosoph/flow-like) ecosystem**. A Rust-powered visual workflow engine that runs on your device. See the [main repository](https://github.com/Rheosoph/flow-like) for the full platform.

---

## Installation

```bash
uv add flow-like
```

For LanceDB support:

```bash
uv add 'flow-like[lance]'
```

## Quick Start

```python
from flow_like import FlowLikeClient

# Auth via environment variables:
#   FLOW_LIKE_BASE_URL - API base URL (required)
#   FLOW_LIKE_PAT      - Personal Access Token (pat_xxx.yyy)
#   FLOW_LIKE_API_KEY   - API Key (flk_app.key.secret)
client = FlowLikeClient()

# Or provide credentials explicitly
client = FlowLikeClient(
    base_url="https://api.flow-like.com",
    pat="pat_xxx.yyy",
)

# Auto-detect token type
client = FlowLikeClient(
    base_url="https://api.flow-like.com",
    token="pat_xxx.yyy",  # or "flk_app.key.secret"
)
```

## Usage

### Trigger Workflow

```python
# Synchronous (SSE streaming); node_id specifies the start node
for event in client.trigger_workflow("app-id", "board-id", "start-node-id", {"key": "value"}):
    print(event.data)

# Asynchronous invocation
result = client.trigger_workflow_async("app-id", "board-id", "start-node-id", {"key": "value"})
print(result.run_id, result.poll_token)
```

### Trigger Event

```python
for event in client.trigger_event("app-id", "event-id", {"key": "value"}):
    print(event.data)

result = client.trigger_event_async("app-id", "event-id")
```

### File Management

```python
# List files
files = client.list_files("app-id")

# Upload
with open("data.csv", "rb") as file:
    client.upload_file("app-id", file, key="imports/data.csv")

# Download
content = client.download_file("app-id", "path/to/file.csv")

# Delete
client.delete_file("app-id", "path/to/file.csv")
```

Uploads and downloads first request signed storage URLs, then transfer bytes without
platform credentials. Use `user=True` for user files. `presign_data` returns scoped
storage credentials and a path; `get_upload_urls` and `get_download_urls` return
transfer URLs.

### Database / LanceDB

```python
# Get presigned credentials (resolved to uri + storage_options)
info = client.get_db_credentials("app-id", access_mode="read", scope="project")
print(info.uri, info.storage_options)

# List tables
tables = client.list_tables("app-id")

# Query a table
result = client.query_table("app-id", "my-table", {"filter": "col > 5"})

# Get a LanceDB connection (requires flow-like[lance])
db = client.create_lance_connection("app-id", access_mode="write", scope="project")
```

### Execution Monitoring

```python
status = client.get_run_status("run-id")
print(status.status)

poll = client.poll_execution("poll-token", after_sequence=-1, timeout=30)
for event in poll.events:
    print(event)
# Pass poll.next_sequence as after_sequence on the next poll.

runs = client.list_runs("app-id", "board-id", limit=20)
logs = client.get_run_logs("app-id", "board-id", "run-id", query={"levels": [2, 3, 4]})
summary = client.get_run_summary("app-id", "board-id", "run-id")
```

### Chat Completions

```python
# bit_id identifies the model; use list_llms() to discover available ones
result = client.chat_completions(
    messages=[{"role": "user", "content": "Hello!"}],
    bit_id="bit-id-for-gpt4",
)
print(result.choices)

# Streaming
for event in client.chat_completions(
    messages=[{"role": "user", "content": "Hello!"}],
    bit_id="bit-id-for-gpt4",
    stream=True,
):
    print(event.data)

# Usage tracking
usage = client.get_usage()
print(usage)  # {"llm_price": ..., "embedding_price": ...}
```

### Responses API

Model bits whose provider declares the `Responses` API surface are served by
`/responses` instead of `/chat/completions`.

```python
result = client.responses(
    input="Hello!",
    bit_id="bit-id-for-a-responses-model",
)
print(result["output"])

# Streaming
for event in client.responses(
    input="Hello!",
    bit_id="bit-id-for-a-responses-model",
    stream=True,
):
    print(event.data)
```

### Embeddings

```python
# bit_id identifies the embedding model; use list_embedding_models() to discover
result = client.embed(bit_id="bit-id-for-embedding", input="Hello world")
print(result.embeddings)
```

### Models / Bits

```python
# List available LLMs (remote only)
llms = client.list_llms()
for m in llms:
    print(m.bit_id, m.name, m.provider_name)

# List embedding models (remote only)
embeddings = client.list_embedding_models()

# Search all bits
bits = client.search_bits(search="llama", bit_types=["Llm"])

# Get a specific bit by ID
bit = client.get_bit("some-bit-id")
```

### Board Management

```python
# List boards
boards = client.list_boards("app-id")

# Read a board
board = client.get_board("app-id", "board-id")

# Create / update a board
result = client.upsert_board("app-id", "board-id", name="My Board", description="Does things")
print(result.id)

# Delete a board
client.delete_board("app-id", "board-id")

# Pre-run analysis
prerun = client.prerun_board("app-id", "board-id")
print(prerun.runtime_variables)
```

Publish the stored board with `publish_board_if_changed(app_id, board_id)`. The
returned version identifies the deployed workflow snapshot. Lifecycle methods such
as `upsert_event`, `apply_flowscript`, and `table_reference_action` accept a backend
JSON `body`; their optional `params` argument holds query parameters.

The client also exposes pages, routes, widgets, connections, packages, roles, team
membership, API keys, and device hub management. Device REST methods manage hub
records and encrypted envelopes. Live device execution still requires the encrypted
management protocol and a controller identity.

### HTTP Sink

```python
response = client.trigger_http_sink("app-id", "my/webhook/path", method="POST", body={"data": 1})
```

### App Management

```python
apps = client.list_apps()
app = client.get_app("app-id")
new_app = client.create_app("My App", "Description")
```

### Health Check

```python
health = client.health()
print(health.healthy)
```

## Async Support

All methods have async counterparts prefixed with `a`:

```python
import asyncio
from flow_like import FlowLikeClient

async def main():
    async with FlowLikeClient(base_url="https://api.flow-like.com", pat="pat_xxx.yyy") as client:
        apps = await client.alist_apps()
        status = await client.ahealth()

asyncio.run(main())
```

## Authentication

| Type | Prefix | Header | Env Variable |
|------|--------|--------|-------------|
| PAT | `pat_` | `Authorization: pat_{id}.{secret}` | `FLOW_LIKE_PAT` |
| API Key | `flk_` | `X-API-Key: flk_{app_id}.{key_id}.{secret}` | `FLOW_LIKE_API_KEY` |

## LangChain Integration

Optional LangChain-compatible wrappers are available.

```bash
uv add flow-like[langchain]
```

```python
from flow_like.langchain import FlowLikeChatModel, FlowLikeEmbeddings

# Option 1: Factory methods from an existing client (recommended)
chat_model = client.as_langchain_chat(
    bit_id="your-model-bit-id",
    temperature=0.7,
    max_tokens=1024,
)
embeddings = client.as_langchain_embeddings(bit_id="your-embedding-bit-id")

# Option 2: Standalone (requires base_url + token)
chat_model2 = FlowLikeChatModel(
    base_url="https://api.flow-like.com",
    token="pat_myid.mysecret",
    bit_id="your-model-bit-id",
    temperature=0.7,
    max_tokens=1024,
)

response = chat_model.invoke("Hello, how are you?")

embeddings = FlowLikeEmbeddings(
    base_url="https://api.flow-like.com",
    token="pat_myid.mysecret",
    bit_id="your-embedding-bit-id",
)

vectors = embeddings.embed_documents(["Hello world", "Goodbye world"])
query_vector = embeddings.embed_query("search query")
```

## License

MIT
