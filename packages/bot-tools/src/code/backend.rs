use std::time::Duration;

use async_trait::async_trait;
use flow_like_types::Bytes;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanguageSupport {
    pub id: String,
    pub installed: bool,
    pub download_bytes: u64,
    /// Library sets `RunRequest::packages` can load (empty: the language has none).
    pub packages: Vec<String>,
    pub packages_installed: bool,
    pub packages_download_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct RunRequest {
    pub language: String,
    pub code: String,
    /// Staged under `/flow/in/<path>`.
    pub files: Vec<(String, Bytes)>,
    /// Empty: no network at all.
    pub network_hosts: Vec<String>,
    pub wall_time: Duration,
    pub memory_bytes: usize,
    pub max_input_bytes: u64,
    /// Library sets to load, from `LanguageSupport::packages`.
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Ok,
    Error,
    Timeout,
    OutOfMemory,
    FuelExhausted,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct RunResponse {
    pub status: RunStatus,
    pub outputs: Value,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    /// New or changed files, relative to `/flow`: `out/<path>` or `in/<path>`.
    pub files: Vec<(String, Bytes)>,
    pub truncated: bool,
    pub duration_ms: u64,
}

/// The sandbox behind `code_run`. The code-interpreter crate implements it (feature `code`).
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// The languages this host can run; empty when it cannot run code at all.
    fn languages(&self) -> Vec<LanguageSupport>;

    async fn run(&self, request: RunRequest) -> RunResponse;
}
