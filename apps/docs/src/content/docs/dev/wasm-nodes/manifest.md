---
title: Package Manifest
description: Complete reference for WASM package manifest files
sidebar:
  order: 2
---

Every WASM package requires a `manifest.toml` file that declares its metadata and permissions. Nodes are automatically extracted from the WASM binary during compilation — they do not need to be declared in the manifest.

## Minimal Example

```toml
manifest_version = 1
id = "com.example.hello"
name = "Hello World"
version = "1.0.0"
description = "A simple hello world node"
```

## Full Example

```toml
manifest_version = 1
id = "com.example.google-drive"
name = "Google Drive Integration"
version = "2.1.0"
description = "Read and write files to Google Drive"
license = "MIT"
repository = "https://github.com/example/flow-like-gdrive"
homepage = "https://example.com/flow-like-gdrive"
keywords = ["google", "drive", "cloud", "storage"]
min_flow_like_version = "0.5.0"

[[authors]]
name = "Jane Developer"
email = "jane@example.com"
url = "https://jane.dev"

[permissions]
memory = "standard"
timeout = "extended"
variables = true
cache = true
streaming = true
models = false
a2ui = false

[permissions.network]
http_enabled = true
allowed_hosts = ["*.googleapis.com", "accounts.google.com"]
websocket_enabled = false

[permissions.filesystem]
node_storage = true
user_storage = false
upload_dir = true
cache_dir = true

[[permissions.oauth_scopes]]
provider = "google"
scopes = [
  "https://www.googleapis.com/auth/drive.readonly",
  "https://www.googleapis.com/auth/drive.file"
]
reason = "Read and write files to Google Drive"
required = true
```

## Root Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `manifest_version` | integer | ✅ | Always `1` for current version |
| `id` | string | ✅ | Unique package ID (reverse domain style) |
| `name` | string | ✅ | Human-readable package name |
| `version` | string | ✅ | Semantic version (e.g., "1.2.3") |
| `description` | string | ✅ | Brief description of the package |
| `license` | string | | SPDX license identifier |
| `repository` | string | | Source code repository URL |
| `homepage` | string | | Package homepage URL |
| `keywords` | string[] | | Search keywords |
| `min_flow_like_version` | string | | Minimum required Flow-Like version |
| `wasm_path` | string | | Path to WASM file (for local dev) |
| `wasm_hash` | string | | SHA-256 hash for integrity |

## Authors

```toml
[[authors]]
name = "Your Name"
email = "you@example.com"  # optional
url = "https://your.site"   # optional
```

## Permissions

### Resource Tiers

```toml
[permissions]
memory = "standard"   # minimal, light, standard, heavy, intensive, large, huge, extreme, maximum
timeout = "standard"  # quick, standard, extended, long_running, very_long, maximum
```

**Memory Tiers:**

| Tier | Memory | Description |
|------|--------|-------------|
| `minimal` | 16 MB | Simple operations |
| `light` | 32 MB | Basic processing |
| `standard` | 64 MB | Most nodes (default) |
| `heavy` | 128 MB | Data processing |
| `intensive` | 256 MB | ML, large datasets |
| `large` | 512 MB | Large model inference |
| `huge` | 1 GB | Very large datasets |
| `extreme` | 2 GB | Heavy computation |
| `maximum` | 4 GB | Maximum allocation |

**Timeout Tiers:**

| Tier | Duration | Description |
|------|----------|-------------|
| `quick` | 5s | Fast operations |
| `standard` | 30s | Most nodes (default) |
| `extended` | 60s | API calls |
| `long_running` | 5min | ML inference |
| `very_long` | 10min | Heavy processing |
| `maximum` | 30min | Maximum duration |

### Capability Flags

```toml
[permissions]
variables = true    # Access execution variables
cache = true        # Access execution cache
streaming = true    # Stream output to UI
a2ui = true         # Adaptive UI rendering
models = true       # Access LLM/model providers
```

### Network Permissions

```toml
[permissions.network]
http_enabled = true
allowed_hosts = ["api.example.com", "*.googleapis.com"]
websocket_enabled = false
```

- `allowed_hosts` supports wildcards (`*`)
- Empty `allowed_hosts` with `http_enabled = true` allows all hosts

### Filesystem Permissions

```toml
[permissions.filesystem]
node_storage = true   # Per-node persistent storage
user_storage = false  # Per-user storage
upload_dir = true     # Access uploaded files
cache_dir = true      # Temporary cache storage
```

### OAuth Scopes

```toml
[[permissions.oauth_scopes]]
provider = "google"
scopes = ["https://www.googleapis.com/auth/drive.readonly"]
reason = "Read files from your Google Drive"
required = true
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `provider` | string | ✅ | OAuth provider ID |
| `scopes` | string[] | ✅ | Required OAuth scopes |
| `reason` | string | ✅ | User-facing explanation |
| `required` | boolean | | If false, node works without OAuth |

**Supported Providers:**

- `google` - Google OAuth 2.0
- `github` - GitHub OAuth
- `microsoft` - Microsoft/Azure AD
- `slack` - Slack OAuth
- `discord` - Discord OAuth
- Custom providers via Flow-Like configuration

## Node Discovery

Nodes are **not** declared in the manifest. Instead, they are automatically extracted from the WASM binary:

- **Remote (registry):** The backend compiles the WASM and calls `get_nodes()` to discover all nodes.
- **Local (desktop):** The runtime calls `get_nodes()` on the loaded WASM module.

This ensures the node catalog always matches the actual WASM code and prevents manifests from misrepresenting capabilities.

## Validation

The manifest is validated when:

1. **Loading** — Package won't load if invalid
2. **Publishing** — Registry rejects invalid manifests

Common validation errors:

| Error | Cause |
|-------|-------|
| `Package ID is required` | Missing `id` field |
| `Invalid memory tier` | Unknown value for `memory` |

## Best Practices

### Package IDs

Use reverse domain notation:

```toml
# Good
id = "com.yourcompany.package-name"
id = "io.github.username.package-name"

# Avoid
id = "my-package"
id = "package_v2"
```

### Minimal Permissions

Only request what you need:

```toml
# Good - specific hosts
[permissions.network]
http_enabled = true
allowed_hosts = ["api.openai.com"]

# Avoid - all hosts when not needed
[permissions.network]
http_enabled = true
allowed_hosts = []
```

### Clear OAuth Reasons

Help users understand why you need access:

```toml
# Good
reason = "Read your calendar events to schedule workflows"

# Avoid
reason = "Google access"
```

### Semantic Versioning

Follow semver for predictable updates:

- `1.0.0` → `1.0.1` — Bug fixes
- `1.0.0` → `1.1.0` — New features, backward compatible
- `1.0.0` → `2.0.0` — Breaking changes
