use super::MetadataState;
use crate::error::{WasmError, WasmResult};
use wasmtime::component::{ComponentType, Lift, Linker, Lower};

#[derive(ComponentType, Lift, Lower)]
#[component(record)]
struct StorageError {
    code: String,
    message: String,
    retryable: bool,
}

fn denied() -> StorageError {
    StorageError {
        code: "permission_denied".into(),
        message: "Storage is unavailable during metadata extraction.".into(),
        retryable: false,
    }
}

pub(super) fn register(linker: &mut Linker<MetadataState>) -> WasmResult<()> {
    let result = (|| -> wasmtime::Result<()> {
        let mut instance = linker.instance("flow-like:node/logging@0.1.0")?;
        instance.func_wrap(
            "log",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (u8, String)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        let mut instance = linker.instance("flow-like:node/pins@0.1.0")?;
        instance.func_wrap(
            "get-input",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "set-output",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        instance.func_wrap(
            "activate-exec",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        let mut instance = linker.instance("flow-like:node/variables@0.1.0")?;
        instance.func_wrap(
            "get-var",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "set-var",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        instance.func_wrap(
            "delete-var",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        instance.func_wrap(
            "has-var",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/cache@0.1.0")?;
        instance.func_wrap(
            "cache-get",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "cache-set",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        instance.func_wrap(
            "cache-delete",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        instance.func_wrap(
            "cache-has",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/streaming@0.1.0")?;
        instance.func_wrap(
            "emit",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        instance.func_wrap(
            "text",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<()> { Ok(()) },
        )?;
        let mut instance = linker.instance("flow-like:node/metadata@0.1.0")?;
        instance.func_wrap(
            "get-node-id",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(String,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "get-run-id",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(String,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "get-app-id",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(String,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "get-board-id",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(String,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "get-user-id",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(String,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "time-now",
            |_: wasmtime::StoreContextMut<'_, MetadataState>, _: ()| -> wasmtime::Result<(u64,)> {
                Ok((super::observations::time_millis(),))
            },
        )?;
        instance.func_wrap(
            "random",
            |_: wasmtime::StoreContextMut<'_, MetadataState>, _: ()| -> wasmtime::Result<(f64,)> {
                Ok((super::observations::component_random(),))
            },
        )?;
        instance.func_wrap(
            "new-resource-handle",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "is-streaming",
            |_: wasmtime::StoreContextMut<'_, MetadataState>, _: ()| -> wasmtime::Result<(bool,)> {
                Ok(Default::default())
            },
        )?;
        instance.func_wrap(
            "get-log-level",
            |_: wasmtime::StoreContextMut<'_, MetadataState>, _: ()| -> wasmtime::Result<(u8,)> {
                Ok(Default::default())
            },
        )?;
        let mut instance = linker.instance("flow-like:node/storage@0.1.0")?;
        instance.func_wrap(
            "storage-dir",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (bool,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "upload-dir",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "cache-dir",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (bool, bool)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "user-dir",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (bool,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "read-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<Vec<u8>>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "copy-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "file-url",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "write-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Vec<u8>)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "write-file-start",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, u64)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "write-file-chunk",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Vec<u8>)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "write-file-finish",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "list-files",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/storage-ops@0.1.0")?;
        instance.func_wrap(
            "stat-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<String, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        instance.func_wrap(
            "read-file-range",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, u64, u64, Option<String>)|
             -> wasmtime::Result<(Result<Vec<u8>, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        instance.func_wrap(
            "list-files-page",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<String, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        instance.func_wrap(
            "close-list",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<(), StorageError>,)> { Ok((Err(denied()),)) },
        )?;
        instance.func_wrap(
            "write-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Vec<u8>, String)|
             -> wasmtime::Result<(Result<String, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        instance.func_wrap(
            "write-file-start",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Option<u64>)|
             -> wasmtime::Result<(Result<String, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        instance.func_wrap(
            "write-file-chunk",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Vec<u8>)|
             -> wasmtime::Result<(Result<(), StorageError>,)> { Ok((Err(denied()),)) },
        )?;
        instance.func_wrap(
            "write-file-finish",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<String, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        instance.func_wrap(
            "write-file-abort",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<(), StorageError>,)> { Ok((Err(denied()),)) },
        )?;
        instance.func_wrap(
            "delete-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<(), StorageError>,)> { Ok((Err(denied()),)) },
        )?;
        instance.func_wrap(
            "move-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String, bool)|
             -> wasmtime::Result<(Result<(), StorageError>,)> { Ok((Err(denied()),)) },
        )?;
        instance.func_wrap(
            "copy-file",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String, bool)|
             -> wasmtime::Result<(Result<(), StorageError>,)> { Ok((Err(denied()),)) },
        )?;
        instance.func_wrap(
            "file-url",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Result<Option<String>, StorageError>,)> {
                Ok((Err(denied()),))
            },
        )?;
        let mut instance = linker.instance("flow-like:node/models@0.1.0")?;
        instance.func_wrap(
            "embed-text",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "embed-text-query",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "embed-text-document",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "embed-image",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Vec<u8>)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "llm-prompt",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String, bool)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "llm-prompt-stream",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/auth@0.1.0")?;
        instance.func_wrap(
            "get-oauth-token",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "has-oauth-token",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/http@0.1.0")?;
        instance.func_wrap(
            "request",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (u8, String, String, Option<Vec<u8>>)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/schema@0.1.0")?;
        instance.func_wrap(
            "get-type-schema",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             (name,): (String,)|
             -> wasmtime::Result<(Option<String>,)> {
                Ok((super::schemas::get(&name),))
            },
        )?;
        instance.func_wrap(
            "list-types",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: ()|
             -> wasmtime::Result<(Option<String>,)> {
                Ok((Some(serde_json::to_string(super::schemas::TYPES)?),))
            },
        )?;
        let mut instance = linker.instance("flow-like:node/image@0.1.0")?;
        instance.func_wrap(
            "from-bytes",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (Vec<u8>, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "to-bytes",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(Option<Vec<u8>>,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/db@0.1.0")?;
        instance.func_wrap(
            "query",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (u32, String, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        let mut instance = linker.instance("flow-like:node/websocket@0.1.0")?;
        instance.func_wrap(
            "connect",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, String)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "send",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, Vec<u8>, bool)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "receive",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String, u32)|
             -> wasmtime::Result<(Option<String>,)> { Ok(Default::default()) },
        )?;
        instance.func_wrap(
            "close",
            |_: wasmtime::StoreContextMut<'_, MetadataState>,
             _: (String,)|
             -> wasmtime::Result<(bool,)> { Ok(Default::default()) },
        )?;
        Ok(())
    })();
    result.map_err(|error| WasmError::Initialization(error.to_string()))
}
