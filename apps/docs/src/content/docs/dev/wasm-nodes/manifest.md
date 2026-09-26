---
title: Package Manifest
description: Reference for Flow-Like WASM package manifests
sidebar:
  order: 2
---

Every WASM package should include a `flow-like.toml` file beside its project
sources. The manifest describes the package and the package-wide settings a
node cannot state in code: resource limits, the outbound host allowlist, and
OAuth scopes. Node definitions and execution permissions come from the compiled
WASM binary.

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
| `DatabaseRead` | Read from wired database and SQL session pins |
| `DatabaseWrite` | Modify rows through wired database pins |
| `Variables` | Read and write flow variables |
| `Cache` | Read and write execution cache |
| `Streaming` | Stream events or text |
| `Models` | Use model-provider host functions |
| `A2ui` | Use A2UI host functions |
| `OAuth` | Access OAuth tokens |
| `Functions` | Call functions or subflows |

Language SDKs expose the same serialized permission labels, such as
`"network:http"`, `"storage:write"`, and `"streaming"`.

The sandbox grants each node exactly the capabilities that node declares. The
manifest cannot add or remove them.

## Host allowlist and OAuth scopes

Besides the resource tiers, `[permissions]` holds two package-wide settings:

```toml
[permissions.network]
allowed_hosts = ["api.example.com"]

[[permissions.oauth_scopes]]
provider = "google"
scopes = ["https://www.googleapis.com/auth/calendar.events"]
reason = "Create calendar events"
required = true
```

| Field | Description |
| --- | --- |
| `network.allowed_hosts` | Outbound host allowlist for every node in the package. An empty or missing list means unrestricted hosts |
| `oauth_scopes[].provider` | OAuth provider ID |
| `oauth_scopes[].scopes` | Scopes requested from that provider |
| `oauth_scopes[].reason` | Why the package needs them; shown in the store |
| `oauth_scopes[].required` | Marks the entry as required in the store; defaults to `false` |

WASM node definitions carry no OAuth providers or scopes today, so these
authored entries are the only source for the store's OAuth listing.

:::caution
`allowed_hosts` is not a complete egress control. The desktop loader for
installed packages copies it, together with the memory and timeout tiers, into
the execution configuration of every node it loads from the package. It is
checked for WebSocket connects and for WASI sockets, where the destination IP
address is compared with the list. A non-empty list also keeps the standard
`wasi:http` interface from being linked, because that path cannot enforce it.
The Flow-Like HTTP host function does not consult the list today.

The server executor applies neither the manifest tiers nor `allowed_hosts`. It
runs nodes with their declared capabilities and the runtime default limits.
:::

## Capability flags are derived

The typed manifest still has capability fields: `network.http_enabled`,
`websocket_enabled`, `tcp_enabled`, `udp_enabled`, `dns_enabled`,
`filesystem.node_storage`, `user_storage`, `upload_dir`, `cache_dir`,
`database.read`, `database.write`, `variables`, `cache`, `streaming`, `a2ui`,
and `models`. Do not author them. They hold the capability listing shown in the
store, and the registry derives them from the compiled node definitions when a
version is compiled and again when it is approved. Authored values are
replaced.

The desktop app derives the same listing when a developer project is loaded and
logs a warning for every authored flag that no node backs.

A package whose nodes declare `StorageRead` or `StorageWrite` lists all four
storage flags, because the sandbox gates the node, user, upload, and cache
directories on a single storage capability.

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
