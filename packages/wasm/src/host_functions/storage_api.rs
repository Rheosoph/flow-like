//! Shared authorization and wire handling for the opt-in storage operations.

use super::storage::StorageFlowPath;
use super::storage_ops::{
    self as ops, FileStat, ListPage, ListPageRequest, StorageError, StorageResult, WriteOptions,
};
use super::{storage, HostState, StorageContext};
use crate::limits::WasmCapabilities;
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

pub(crate) fn context(
    host: &HostState,
    capabilities: WasmCapabilities,
) -> StorageResult<&StorageContext> {
    if !host.has_capability(capabilities) {
        return Err(StorageError::new(
            "permission_denied",
            "The node lacks the required storage permission.",
        ));
    }
    host.storage_context.as_ref().ok_or_else(|| {
        StorageError::new(
            "permission_denied",
            "No storage context is available for this invocation.",
        )
    })
}

pub(crate) fn parse<T: DeserializeOwned>(value: &str) -> StorageResult<T> {
    if value.len() > 64 * 1024 {
        return Err(StorageError::resource_limit(
            "Storage metadata requests must be at most 64 KiB.",
        ));
    }
    serde_json::from_str(value).map_err(|error| {
        StorageError::invalid_argument(format!("Invalid storage request: {error}"))
    })
}

pub(crate) fn json_result<T: Serialize>(value: StorageResult<T>) -> StorageResult<String> {
    serde_json::to_string(&value?).map_err(|error| {
        StorageError::new("io", format!("Cannot serialize storage result: {error}"))
    })
}

pub(crate) async fn stat(host: &HostState, path: &str) -> StorageResult<FileStat> {
    ops::stat_file(
        context(host, WasmCapabilities::STORAGE_READ)?,
        &parse(path)?,
    )
    .await
}

pub(crate) async fn read_range(
    host: &HostState,
    path: &str,
    offset: u64,
    length: u64,
    if_match: Option<&str>,
) -> StorageResult<Vec<u8>> {
    ops::read_file_range(
        context(host, WasmCapabilities::STORAGE_READ)?,
        &parse(path)?,
        offset,
        length,
        if_match,
    )
    .await
}

pub(crate) async fn list_page(host: &HostState, request: &str) -> StorageResult<ListPage> {
    let request: ListPageRequest = parse(request)?;
    ops::list_files_page(
        context(host, WasmCapabilities::STORAGE_READ)?,
        &host.storage_listings,
        &request,
    )
    .await
}

pub(crate) fn close_list(host: &HostState, cursor: &str) -> StorageResult<()> {
    context(host, WasmCapabilities::STORAGE_READ)?;
    ops::close_list(&host.storage_listings, cursor)
}

pub(crate) async fn write_file(
    host: &HostState,
    path: &str,
    data: Vec<u8>,
    options: &str,
) -> StorageResult<FileStat> {
    let options: WriteOptions = parse(options)?;
    ops::write_file(
        context(host, WasmCapabilities::STORAGE_WRITE)?,
        &parse(path)?,
        data,
        options,
    )
    .await
}

pub(crate) async fn write_start(
    host: &HostState,
    path: &str,
    total_size: Option<u64>,
) -> StorageResult<String> {
    let path: StorageFlowPath = parse(path)?;
    storage::start_write(
        context(host, WasmCapabilities::STORAGE_WRITE)?,
        &host.pending_writes,
        path,
        total_size,
    )
    .await
}

pub(crate) async fn write_chunk(
    host: &HostState,
    write_id: &str,
    data: Vec<u8>,
) -> StorageResult<()> {
    context(host, WasmCapabilities::STORAGE_WRITE)?;
    storage::append_chunk(&host.pending_writes, write_id, data).await
}

pub(crate) async fn write_finish(host: &HostState, write_id: &str) -> StorageResult<FileStat> {
    storage::finish_write(
        context(host, WasmCapabilities::STORAGE_WRITE)?,
        &host.pending_writes,
        write_id,
    )
    .await
}

pub(crate) async fn write_abort(host: &HostState, write_id: &str) -> StorageResult<()> {
    context(host, WasmCapabilities::STORAGE_WRITE)?;
    storage::abort_write(&host.pending_writes, write_id).await
}

pub(crate) async fn delete(host: &HostState, path: &str) -> StorageResult<()> {
    ops::delete_file(
        context(host, WasmCapabilities::STORAGE_DELETE)?,
        &parse(path)?,
    )
    .await
}

pub(crate) async fn copy(
    host: &HostState,
    source: &str,
    destination: &str,
    overwrite: bool,
) -> StorageResult<()> {
    ops::copy_file(
        context(
            host,
            WasmCapabilities::STORAGE_READ | WasmCapabilities::STORAGE_WRITE,
        )?,
        &parse(source)?,
        &parse(destination)?,
        overwrite,
    )
    .await
}

pub(crate) async fn move_file(
    host: &HostState,
    source: &str,
    destination: &str,
    overwrite: bool,
) -> StorageResult<()> {
    ops::move_file(
        context(
            host,
            WasmCapabilities::STORAGE_READ
                | WasmCapabilities::STORAGE_WRITE
                | WasmCapabilities::STORAGE_DELETE,
        )?,
        &parse(source)?,
        &parse(destination)?,
        overwrite,
    )
    .await
}

pub(crate) async fn file_url(host: &HostState, path: &str) -> StorageResult<Option<String>> {
    ops::file_url(
        context(host, WasmCapabilities::STORAGE_READ)?,
        &parse(path)?,
    )
    .await
}

fn field<'a>(request: &'a Value, name: &str) -> StorageResult<&'a Value> {
    request.get(name).ok_or_else(|| {
        StorageError::invalid_argument(format!("Missing storage request field '{name}'."))
    })
}

fn string<'a>(request: &'a Value, name: &str) -> StorageResult<&'a str> {
    field(request, name)?
        .as_str()
        .ok_or_else(|| StorageError::invalid_argument(format!("'{name}' must be a string.")))
}

fn number(request: &Value, name: &str) -> StorageResult<u64> {
    field(request, name)?.as_u64().ok_or_else(|| {
        StorageError::invalid_argument(format!("'{name}' must be an unsigned integer."))
    })
}

fn data(request: &Value) -> StorageResult<Vec<u8>> {
    let value = string(request, "data_base64")?;
    if value.len() > storage::MAX_STORAGE_FILE_SIZE.div_ceil(3) * 4 {
        return Err(StorageError::resource_limit(
            "Use chunks of at most 10 MiB.",
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| StorageError::invalid_argument("Invalid data_base64."))
}

fn value<T: Serialize>(result: StorageResult<T>) -> StorageResult<Value> {
    serde_json::to_value(result?).map_err(|error| StorageError::new("io", error.to_string()))
}

/// The core-module ABI uses JSON because it has no component-model records.
pub(crate) async fn request(host: &HostState, request: &Value) -> StorageResult<Value> {
    match string(request, "op")? {
        "stat_file" => value(stat(host, &field(request, "path")?.to_string()).await),
        "read_file_range" => {
            let bytes = read_range(
                host,
                &field(request, "path")?.to_string(),
                number(request, "offset")?,
                number(request, "length")?,
                request
                    .get("if_match")
                    .filter(|v| !v.is_null())
                    .map(|_| string(request, "if_match"))
                    .transpose()?,
            )
            .await?;
            Ok(json!({ "data_base64": base64::engine::general_purpose::STANDARD.encode(bytes) }))
        }
        "list_files_page" => value(list_page(host, &request.to_string()).await),
        "close_list" => value(close_list(host, string(request, "cursor")?)),
        "write_file" => value(
            write_file(
                host,
                &field(request, "path")?.to_string(),
                data(request)?,
                &request
                    .get("options")
                    .cloned()
                    .unwrap_or_else(|| json!({}))
                    .to_string(),
            )
            .await,
        ),
        "write_file_start" => {
            let total = request
                .get("total_size")
                .filter(|v| !v.is_null())
                .map(|_| number(request, "total_size"))
                .transpose()?;
            value(write_start(host, &field(request, "path")?.to_string(), total).await)
        }
        "write_file_chunk" => {
            value(write_chunk(host, string(request, "write_id")?, data(request)?).await)
        }
        "write_file_finish" => value(write_finish(host, string(request, "write_id")?).await),
        "write_file_abort" => value(write_abort(host, string(request, "write_id")?).await),
        "delete_file" => value(delete(host, &field(request, "path")?.to_string()).await),
        "copy_file" | "move_file" => {
            let source = field(request, "source")?.to_string();
            let destination = field(request, "destination")?.to_string();
            let overwrite = match request.get("overwrite") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err(StorageError::invalid_argument(
                        "'overwrite' must be a boolean.",
                    ))
                }
            };
            if string(request, "op")? == "copy_file" {
                value(copy(host, &source, &destination, overwrite).await)
            } else {
                value(move_file(host, &source, &destination, overwrite).await)
            }
        }
        "file_url" => value(file_url(host, &field(request, "path")?.to_string()).await),
        _ => Err(StorageError::invalid_argument("Unknown storage operation.")),
    }
}

pub(crate) fn envelope(result: StorageResult<Value>) -> Vec<u8> {
    match result {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(error) => json!({ "ok": false, "error": error }),
    }
    .to_string()
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::state::FlowLikeStores;
    use flow_like_storage::{
        files::store::FlowLikeStore,
        object_store::{memory::InMemory, path::Path},
    };
    use std::sync::Arc;

    fn host(capabilities: WasmCapabilities) -> (HostState, Value) {
        let mut host = HostState::new(capabilities);
        let context = StorageContext {
            stores: FlowLikeStores {
                app_storage_store: Some(FlowLikeStore::Memory(Arc::new(InMemory::new()))),
                ..Default::default()
            },
            store_cache: Default::default(),
            credentials_store: None,
            app_id: "a".into(),
            board_dir: Path::from("apps/a"),
            board_id: "board".into(),
            node_id: "node".into(),
            sub: "user".into(),
        };
        let mut path = context
            .dir_flow_path("storage", context.get_storage_dir(false))
            .unwrap();
        path.path = path.object_path().join("file.bin").to_string();
        host.storage_context = Some(context);
        (host, serde_json::to_value(path).unwrap())
    }

    #[tokio::test]
    async fn json_transport_roundtrips_binary_and_preserves_conditional_conflicts() {
        let (host, path) = host(WasmCapabilities::STORAGE_READ | WasmCapabilities::STORAGE_WRITE);
        let bytes = vec![0, 128, 255, 13, 10];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let written = request(&host, &json!({"op":"write_file", "path":path, "data_base64":encoded, "options":{"create_only":true}})).await.unwrap();
        assert_eq!(written["size"], bytes.len());
        let conflict = request(&host, &json!({"op":"write_file", "path":path, "data_base64":"", "options":{"if_match":"stale"}})).await.unwrap_err();
        let error: Value = serde_json::from_slice(&envelope(Err(conflict))).unwrap();
        assert_eq!(error["ok"], false);
        assert_eq!(error["error"]["code"], "conflict");
        assert_eq!(error["error"]["retryable"], false);
        let read = request(&host, &json!({"op":"read_file_range", "path":path, "offset":0, "length":5, "if_match":written["e_tag"]})).await.unwrap();
        assert_eq!(read["data_base64"], encoded);
        assert_eq!(request(&host, &json!({"op":"read_file_range", "path":path, "offset":0, "length":5, "if_match":123})).await.unwrap_err().code, "invalid_argument");
    }

    #[tokio::test]
    async fn json_transport_enforces_capabilities_and_scope() {
        let (host, path) = host(WasmCapabilities::STORAGE_READ);
        assert_eq!(
            request(
                &host,
                &json!({"op":"write_file", "path":path, "data_base64":""})
            )
            .await
            .unwrap_err()
            .code,
            "permission_denied"
        );
        let mut outside = path.clone();
        outside["path"] = json!("apps/another-app/storage/secret");
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":outside}))
                .await
                .unwrap_err()
                .code,
            "permission_denied"
        );
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":path}))
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
        assert!(host.pending_writes.read().is_empty());
    }

    #[tokio::test]
    async fn delete_and_move_require_delete_capability_before_mutating() {
        let (mut host, path) =
            host(WasmCapabilities::STORAGE_READ | WasmCapabilities::STORAGE_WRITE);
        let mut destination = path.clone();
        destination["path"] = json!("apps/a/storage/destination.bin");
        request(
            &host,
            &json!({"op":"write_file", "path":path, "data_base64":"AQI="}),
        )
        .await
        .unwrap();
        for operation in [
            json!({"op":"delete_file", "path":path}),
            json!({"op":"move_file", "source":path, "destination":destination}),
        ] {
            assert_eq!(
                request(&host, &operation).await.unwrap_err().code,
                "permission_denied"
            );
        }
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":path}))
                .await
                .unwrap()["size"],
            2
        );
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":destination}))
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
        request(
            &host,
            &json!({"op":"copy_file", "source":path, "destination":destination}),
        )
        .await
        .unwrap();
        host.capabilities = WasmCapabilities::STORAGE_DELETE;
        assert_eq!(
            request(
                &host,
                &json!({"op":"move_file", "source":path, "destination":destination})
            )
            .await
            .unwrap_err()
            .code,
            "permission_denied"
        );
        request(&host, &json!({"op":"delete_file", "path":path}))
            .await
            .unwrap();
        host.capabilities = WasmCapabilities::STORAGE_READ;
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":path}))
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":destination}))
                .await
                .unwrap()["size"],
            2
        );
    }

    #[tokio::test]
    async fn json_stream_requires_complete_length_and_supports_abort() {
        let (host, path) = host(WasmCapabilities::STORAGE_READ | WasmCapabilities::STORAGE_WRITE);
        let id = request(
            &host,
            &json!({"op":"write_file_start", "path":path, "total_size":3}),
        )
        .await
        .unwrap();
        request(
            &host,
            &json!({"op":"write_file_chunk", "write_id":id, "data_base64":"AQI="}),
        )
        .await
        .unwrap();
        assert_eq!(
            request(&host, &json!({"op":"write_file_finish", "write_id":id}))
                .await
                .unwrap_err()
                .code,
            "incomplete_write"
        );
        assert_eq!(
            request(&host, &json!({"op":"stat_file", "path":path}))
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
        let id = request(&host, &json!({"op":"write_file_start", "path":path}))
            .await
            .unwrap();
        request(&host, &json!({"op":"write_file_abort", "write_id":id}))
            .await
            .unwrap();
        assert!(host.pending_writes.read().is_empty());
    }
}
