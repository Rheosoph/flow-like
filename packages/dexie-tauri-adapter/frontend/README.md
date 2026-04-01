# @flow-like/dexie-tauri-blob-offload

Dexie.js middleware that transparently offloads large values to the native filesystem via Tauri.

## Overview

This package provides a [Dexie](https://dexie.org/) DBCore middleware that intercepts reads and writes. Values exceeding a configurable threshold are stored on disk through blake3 content-addressed hashing, while only a small reference (hash + HMAC) is kept in IndexedDB.

**Requires** the companion Rust plugin [`tauri-plugin-flow-like-dexie-blob-offload`](../backend/) on the Tauri side.

## Installation

```bash
npm install @flow-like/dexie-tauri-blob-offload
```

## Usage

```ts
import Dexie from "dexie";
import { dexieTauriBlobOffload } from "@flow-like/dexie-tauri-blob-offload";

const db = new Dexie("mydb");
db.version(1).stores({ files: "++id, name" });

// Apply the middleware — values > 200 chars/elements are offloaded
db.use(dexieTauriBlobOffload(200));
```

### Rust side (Tauri)

```rust
// src-tauri/src/lib.rs
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_flow_like_dexie_blob_offload::init(None))
        .run(tauri::generate_context!())
        .expect("error running app");
}
```

## API

### `dexieTauriBlobOffload(threshold?: number)`

Returns a Dexie middleware object. The `threshold` parameter controls the minimum size (in characters for strings, elements for arrays) before a value is offloaded. Defaults to `200`.

## How It Works

1. **On write** (`add`/`put`): scans each value for large strings or number arrays. Those are sent to the Rust plugin, which stores them on disk and returns a `{ hash, mac }` reference. The reference replaces the original value in IndexedDB.

2. **On read** (`get`/`getMany`/`query`): scans results for blob references, fetches the data from disk (verifying the HMAC), and replaces references with the original values.

## License

MIT
