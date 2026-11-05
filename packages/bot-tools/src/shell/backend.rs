use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct ShellRequest {
    /// A program name or path; with `script` the shell command line.
    pub program: String,
    pub args: Vec<String>,
    /// Run `program` as a script through `sh -c` (`cmd /C` on Windows).
    pub script: bool,
    /// Relative to the backend's working directory, already confined.
    pub cwd: String,
    pub stdin: Option<String>,
    pub wall_time: Duration,
    /// Per stream; the response keeps the head and the tail within it.
    pub output_bytes: usize,
    /// Added to the scrubbed environment: the bot's secrets by name.
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct ShellResponse {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_omitted: u64,
    pub stderr_omitted: u64,
    pub duration_ms: u64,
    /// The process never started.
    pub spawn_error: Option<String>,
    pub sandboxed: bool,
}

/// The process runner behind `shell_run`. [`super::LocalShell`] implements it.
#[async_trait]
pub trait ShellBackend: Send + Sync {
    /// The bot's working directory on this device.
    fn root(&self) -> PathBuf;

    async fn run(
        &self,
        request: ShellRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ShellResponse;
}
