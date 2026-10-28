//! Opt-in component storage API. The legacy storage instance remains unchanged.

use super::linker::ComponentStoreData;
use crate::error::{WasmError, WasmResult};
use crate::host_functions::{storage_api as api, storage_ops::StorageError};
use wasmtime::component::{ComponentType, Lift, Linker, Lower};

#[derive(ComponentType, Lift, Lower)]
#[component(record)]
struct WitStorageError {
    code: String,
    message: String,
    retryable: bool,
}

impl From<StorageError> for WitStorageError {
    fn from(error: StorageError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        }
    }
}

pub(crate) fn register(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let map_error =
        |error| WasmError::Initialization(format!("Failed to register storage-ops: {error}"));
    let mut storage = linker
        .instance("flow-like:node/storage-ops@0.1.0")
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "stat-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (path,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::json_result(api::stat(host, &path).await);
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "read-file-range",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (path, offset, length, if_match): (String, u64, u64, Option<String>)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result =
                        api::read_range(host, &path, offset, length, if_match.as_deref()).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "list-files-page",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (request,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::json_result(api::list_page(host, &request).await);
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "close-list",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (cursor,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::close_list(host, &cursor);
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "write-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (path, data, options): (String, Vec<u8>, String)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result =
                        api::json_result(api::write_file(host, &path, data, &options).await);
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "write-file-start",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (path, total_size): (String, Option<u64>)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::write_start(host, &path, total_size).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "write-file-chunk",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (write_id, data): (String, Vec<u8>)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::write_chunk(host, &write_id, data).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "write-file-finish",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (write_id,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::json_result(api::write_finish(host, &write_id).await);
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "write-file-abort",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (write_id,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::write_abort(host, &write_id).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "delete-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (path,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::delete(host, &path).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "move-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (source, destination, overwrite): (String, String, bool)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::move_file(host, &source, &destination, overwrite).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "copy-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (source, destination, overwrite): (String, String, bool)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::copy(host, &source, &destination, overwrite).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    storage
        .func_wrap_async(
            "file-url",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (path,): (String,)| {
                Box::new(async move {
                    let host = &store.data().host_state;
                    let result = api::file_url(host, &path).await;
                    Ok((result.map_err(WitStorageError::from),))
                })
            },
        )
        .map_err(map_error)?;
    Ok(())
}
