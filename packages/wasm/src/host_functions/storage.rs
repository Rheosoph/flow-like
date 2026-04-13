//! Storage host functions
//!
//! Provides storage access for WASM modules.

use flow_like_storage::object_store::{path::Path, PutPayload};
use flow_like_types::Bytes;
use std::collections::HashMap;

use super::StorageContext;

pub const MAX_STORAGE_FILE_SIZE: usize = 10 * 1024 * 1024;
pub const MAX_PENDING_WRITES: usize = 8;
pub const MAX_TOTAL_WRITE_SIZE: usize = 512 * 1024 * 1024;

#[derive(Debug)]
pub struct PendingWrite {
    pub flow_path: StorageFlowPath,
    pub buffer: Vec<u8>,
    pub total_size: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct StorageFlowPath {
    pub path: String,
    pub store_ref: String,
    pub cache_store_ref: Option<String>,
}

pub fn validate_path(path: &str) -> bool {
    !path.contains("..") && !path.starts_with('/') && !path.is_empty()
}

pub fn start_write(
    pending: &mut HashMap<String, PendingWrite>,
    flow_path: StorageFlowPath,
    total_size: u64,
) -> Option<String> {
    if pending.len() >= MAX_PENDING_WRITES {
        tracing::warn!("[wasm write-start] rejected: too many pending writes");
        return None;
    }
    if total_size as usize > MAX_TOTAL_WRITE_SIZE {
        tracing::warn!(
            "[wasm write-start] rejected: total_size {} exceeds max {}",
            total_size,
            MAX_TOTAL_WRITE_SIZE
        );
        return None;
    }
    let id = format!(
        "cw_{:x}_{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        pending.len()
    );
    pending.insert(
        id.clone(),
        PendingWrite {
            flow_path,
            buffer: Vec::with_capacity(total_size as usize),
            total_size,
        },
    );
    Some(id)
}

pub fn append_chunk(
    pending: &mut HashMap<String, PendingWrite>,
    write_id: &str,
    data: &[u8],
) -> bool {
    if data.len() > MAX_STORAGE_FILE_SIZE {
        tracing::warn!(
            "[wasm write-chunk] rejected: chunk size {} exceeds max {}",
            data.len(),
            MAX_STORAGE_FILE_SIZE
        );
        return false;
    }
    let Some(pw) = pending.get_mut(write_id) else {
        tracing::warn!("[wasm write-chunk] rejected: unknown write_id {write_id}");
        return false;
    };
    if pw.buffer.len() + data.len() > pw.total_size as usize {
        tracing::warn!("[wasm write-chunk] rejected: would exceed declared total_size");
        return false;
    }
    pw.buffer.extend_from_slice(data);
    true
}

pub async fn finish_write(
    pending: &mut HashMap<String, PendingWrite>,
    write_id: &str,
    storage_ctx: &StorageContext,
) -> bool {
    let Some(pw) = pending.remove(write_id) else {
        tracing::warn!("[wasm write-finish] rejected: unknown write_id {write_id}");
        return false;
    };
    let Some(store) = storage_ctx.resolve_store(&pw.flow_path.store_ref) else {
        tracing::warn!(
            "[wasm write-finish] rejected: unresolved store_ref={}",
            pw.flow_path.store_ref
        );
        return false;
    };
    let path = Path::from(pw.flow_path.path.clone());
    let payload = PutPayload::from_bytes(Bytes::from(pw.buffer));
    match store.as_generic().put(&path, payload).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(
                "[wasm write-finish] put failed for path={} store_ref={}: {e}",
                pw.flow_path.path,
                pw.flow_path.store_ref
            );
            false
        }
    }
}
