# tauri-plugin-flow-like-dexie-blob-offload

A Tauri v2 plugin that transparently offloads large values from Dexie/IndexedDB to the native filesystem.

## Why?

IndexedDB in WebView-based desktop apps (Tauri, Electron) has practical size limits that browsers handle gracefully but embedded WebViews do not. This plugin intercepts Dexie read/write operations via a DBCore middleware and:

- **On write**: Extracts strings and binary arrays exceeding a configurable threshold, stores them as content-addressed files via blake3 hashing, and replaces the values with compact references.
- **On read**: Detects blob references and transparently rehydrates them from the filesystem.

## Security

References use HMAC-blake3 with a per-install random key — forging a valid reference without the key is computationally infeasible. This prevents injection attacks where a backend response could trick the app into reading arbitrary files.

## Usage (Rust)

```rust
// src-tauri/src/lib.rs
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_flow_like_dexie_blob_offload::init(None))
        // ... other plugins
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Pass `Some(path)` to `init()` to override the default storage directory (`{app_data_dir}/blob_store`).

## Usage (TypeScript)

See the companion npm package `@flow-like/dexie-tauri-blob-offload` for the Dexie middleware.
