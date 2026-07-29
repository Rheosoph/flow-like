---
title: Package Manifest
description: Reference for Flow-Like WASM package manifests
sidebar:
  order: 2
---

Every WASM package should include a `flow-like.toml` file beside its project
sources. The manifest describes the package and its resource limits. Node
definitions and execution permissions come from the compiled WASM binary.

## Minimal manifest

```toml title="flow-like.toml"
manifest_version = 1
id = "com.example.hello"
name = "Hello World"
version = "1.0.0"
description = "A simple example package"
wasm_path = "build/node.wasm"

[permissions]
memory = "standard"
timeout = "standard"
```

The desktop developer tools look specifically for `flow-like.toml`.

## Package fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `manifest_version` | integer | Yes | Current schema version: `1` |
| `id` | string | Yes | Stable package ID, preferably reverse-domain notation |
| `name` | string | Yes | Package display name |
| `version` | string | Yes | Package version |
| `description` | string | Yes | Short package description |
| `authors` | array | No | Author records |
| `license` | string | No | SPDX license identifier |
| `repository` | string | No | Source repository URL |
| `homepage` | string | No | Package homepage |
| `keywords` | string array | No | Discovery keywords |
| `primary_category` | string | No | Primary package category |
| `secondary_category` | string | No | Secondary package category |
| `min_flow_like_version` | string | No | Minimum compatible Flow-Like version |
| `wasm_path` | string | No | WASM path relative to the manifest |
| `wasm_hash` | string | No | SHA-256 integrity hash |
| `metadata` | table | No | Additional package metadata |

Authors use TOML array-of-table syntax:

```toml
[[authors]]
name = "Jane Developer"
email = "jane@example.com"
url = "https://example.com"
```

Categories use the enum's uppercase names, for example
`"DOCUMENT_PROCESSING"`, `"WORKFLOW_AUTOMATION"`,
`"INTEGRATION_CONNECTORS"`, `"AI_ML"`, or `"OTHER"`.

## Resource limits

Package resource limits are applied to each node loaded from the package:

```toml
[permissions]
memory = "standard"
timeout = "extended"
```

### Memory tiers

| Tier | Limit |
| --- | --- |
| `minimal` | 16 MB |
| `light` | 32 MB |
| `standard` | 64 MB |
| `heavy` | 128 MB |
| `intensive` | 256 MB |
| `large` | 512 MB |
| `huge` | 1 GB |
| `extreme` | 2 GB |
| `maximum` | 4 GB |

### Timeout tiers

| Tier | Limit |
| --- | --- |
| `quick` | 5 seconds |
| `standard` | 30 seconds |
| `extended` | 60 seconds |
| `long_running` | 5 minutes |
| `very_long` | 10 minutes |
| `maximum` | 30 minutes |

Choose the smallest tier that supports normal operation. A higher tier increases
the maximum available resource; it does not reserve that resource in advance.

## Execution permissions belong to nodes

Each node exports its own permissions from code. For example, a Rust WASM node
that performs an HTTP request and writes storage declares:

```rust
use flow_like_wasm_sdk::NodePermission;

node.add_permission(NodePermission::NetworkHttp);
node.add_permission(NodePermission::StorageWrite);
```

The available node permissions are:

| Permission | Capability |
| --- | --- |
| `NetworkHttp` | Outbound HTTP |
| `NetworkWebsocket` | WebSocket access |
| `NetworkTcp` | TCP sockets |
| `NetworkUdp` | UDP sockets |
| `NetworkDns` | DNS lookups |
| `StorageRead` | Read node/user storage |
| `StorageWrite` | Write and delete node/user storage |
| `Variables` | Read and write flow variables |
| `Cache` | Read and write execution cache |
| `Streaming` | Stream events or text |
| `Models` | Use model-provider host functions |
| `A2ui` | Use A2UI host functions |
| `OAuth` | Access OAuth tokens |
| `Functions` | Call functions or subflows |

Language SDKs expose the same serialized permission labels, such as
`"network:http"`, `"storage:write"`, and `"streaming"`.

:::caution
The typed manifest still parses package-level network, filesystem, OAuth, and
capability fields for compatibility and package inspection. In the current
loader, those capability flags are used while inspecting the module, but they
are not merged into each node's execution security configuration. Execution
capabilities come from the permissions exported by that node; package memory
and timeout limits are layered onto them.

Do not rely on manifest `allowed_hosts` as an execution-time host allowlist.
:::

## Node discovery

The runtime calls the binary's `get_nodes` export and builds the catalog from
the returned definitions. This keeps the visible catalog synchronized with the
code that will actually run.

The current `PackageManifest` type has no `nodes` field. Older templates may
contain `[[nodes]]` tables; TOML deserialization ignores those unknown tables.
They do not register nodes, set permissions, or validate the binary. Remove
them from new manifests to avoid maintaining a second, ineffective definition.

## Complete example

```toml title="flow-like.toml"
manifest_version = 1
id = "com.example.text-tools"
name = "Text Tools"
version = "1.2.0"
description = "Text transformation nodes"
license = "MIT"
repository = "https://github.com/example/text-tools"
homepage = "https://example.com/text-tools"
keywords = ["text", "transform"]
primary_category = "DOCUMENT_PROCESSING"
wasm_path = "build/node.wasm"

[[authors]]
name = "Jane Developer"
email = "jane@example.com"

[permissions]
memory = "light"
timeout = "quick"

[metadata]
support = "https://example.com/support"
```

## Validation and versioning

The parser requires fields with non-optional types, and publish/install
validation additionally checks that `id`, `name`, and `version` are not empty.
The package ID should remain stable across releases. Increment the version when
behavior or pin interfaces change.

Use reverse-domain package IDs to reduce collisions:

```toml
id = "io.github.username.text-tools"
```

The package version is stored as a string. Semantic versioning is recommended,
even though the manifest validator does not currently perform a strict semver
parse.

## Related

- [WASM Nodes Overview](/dev/wasm-nodes/overview/)
- [Rust WASM Nodes](/dev/wasm-nodes/rust/)
- [Sandboxing and Permissions](/dev/wasm-nodes/sandboxing/)
