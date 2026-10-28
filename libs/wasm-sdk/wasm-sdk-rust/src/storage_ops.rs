//! Typed storage operations. Enable the `storage-ops` Cargo feature to import this interface.
use crate::FlowPath;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use crate::flow_like::node::storage_ops as raw;
#[cfg(not(target_arch = "wasm32"))]
use crate::wit_stub::flow_like::node::storage_ops as raw;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl StorageError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for StorageError {}
pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStat {
    #[serde(flatten)]
    pub path: FlowPath,
    pub size: u64,
    pub e_tag: Option<String>,
    pub version: Option<String>,
    pub last_modified: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WriteOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_match: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub create_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
    pub path: FlowPath,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default)]
    pub exclude_directories: Vec<String>,
    #[serde(default = "recursive_default")]
    pub recursive: bool,
}
fn recursive_default() -> bool {
    true
}

impl ListRequest {
    pub fn new(path: FlowPath) -> Self {
        Self {
            path,
            limit: None,
            cursor: None,
            exclude_directories: Vec::new(),
            recursive: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPage {
    pub entries: Vec<FileStat>,
    pub cursor: Option<String>,
}

fn encode(value: &impl Serialize) -> StorageResult<String> {
    serde_json::to_string(value)
        .map_err(|error| StorageError::new("invalid_argument", error.to_string()))
}
fn decode<T: serde::de::DeserializeOwned>(value: String) -> StorageResult<T> {
    serde_json::from_str(&value)
        .map_err(|error| StorageError::new("invalid_response", error.to_string()))
}
fn host_error(error: raw::StorageError) -> StorageError {
    StorageError {
        code: error.code,
        message: error.message,
        retryable: error.retryable,
    }
}

/// Stateless access to the host's invocation-scoped storage operations.
#[derive(Clone, Copy, Debug, Default)]
pub struct Storage;

impl Storage {
    pub fn stat(&self, path: &FlowPath) -> StorageResult<FileStat> {
        decode(raw::stat_file(&encode(path)?).map_err(host_error)?)
    }

    /// Read at most 10 MiB. Use the same ETag across reads to detect concurrent changes.
    pub fn read_range(
        &self,
        path: &FlowPath,
        offset: u64,
        length: u64,
        if_match: Option<&str>,
    ) -> StorageResult<Vec<u8>> {
        raw::read_file_range(&encode(path)?, offset, length, if_match).map_err(host_error)
    }

    /// Cursors belong to the current invocation. Close an abandoned listing explicitly.
    pub fn list_page(&self, request: &ListRequest) -> StorageResult<ListPage> {
        decode(raw::list_files_page(&encode(request)?).map_err(host_error)?)
    }
    pub fn close_list(&self, cursor: &str) -> StorageResult<()> {
        raw::close_list(cursor).map_err(host_error)
    }
    pub fn write(
        &self,
        path: &FlowPath,
        data: &[u8],
        options: &WriteOptions,
    ) -> StorageResult<FileStat> {
        decode(raw::write_file(&encode(path)?, data, &encode(options)?).map_err(host_error)?)
    }
    /// Start an unconditional streaming write. A declared size must match exactly at finish.
    pub fn writer(&self, path: &FlowPath, total_size: Option<u64>) -> StorageResult<StorageWriter> {
        let id = raw::write_file_start(&encode(path)?, total_size).map_err(host_error)?;
        Ok(StorageWriter { id: Some(id) })
    }
    pub fn delete(&self, path: &FlowPath) -> StorageResult<()> {
        raw::delete_file(&encode(path)?).map_err(host_error)
    }
    pub fn move_file(
        &self,
        source: &FlowPath,
        destination: &FlowPath,
        overwrite: bool,
    ) -> StorageResult<()> {
        raw::move_file(&encode(source)?, &encode(destination)?, overwrite).map_err(host_error)
    }
    pub fn copy(
        &self,
        source: &FlowPath,
        destination: &FlowPath,
        overwrite: bool,
    ) -> StorageResult<()> {
        raw::copy_file(&encode(source)?, &encode(destination)?, overwrite).map_err(host_error)
    }
    pub fn file_url(&self, path: &FlowPath) -> StorageResult<Option<String>> {
        raw::file_url(&encode(path)?).map_err(host_error)
    }
}

impl crate::Context {
    pub fn storage_ops(&self) -> Storage {
        Storage
    }
}

/// Dropping an unfinished writer aborts its upload.
#[derive(Debug)]
pub struct StorageWriter {
    id: Option<String>,
}
impl StorageWriter {
    pub fn write_chunk(&mut self, data: &[u8]) -> StorageResult<()> {
        raw::write_file_chunk(self.id.as_deref().expect("writer is open"), data).map_err(host_error)
    }
    pub fn finish(mut self) -> StorageResult<FileStat> {
        let id = self.id.as_deref().expect("writer is open");
        let metadata = raw::write_file_finish(id).map_err(host_error)?;
        self.id = None;
        decode(metadata)
    }
    pub fn abort(mut self) -> StorageResult<()> {
        let result =
            raw::write_file_abort(self.id.as_deref().expect("writer is open")).map_err(host_error);
        if result.is_ok() {
            self.id = None;
        }
        result
    }
}
impl Drop for StorageWriter {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = raw::write_file_abort(&id);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[path = "storage_ops_mock.rs"]
pub mod mock;
