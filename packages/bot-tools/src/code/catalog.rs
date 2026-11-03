use std::sync::Arc;

use async_trait::async_trait;
use flow_like_catalog_code_interpreter::sandbox::{
    Language, NetworkPolicy, SandboxFile, SandboxRequest, SandboxRuntime, SandboxStatus,
    shared_runtime,
};
use flow_like_types::Bytes;

use super::backend::{LanguageSupport, RunRequest, RunResponse, RunStatus, SandboxBackend};

pub struct CatalogSandbox {
    runtime: Arc<SandboxRuntime>,
}

impl CatalogSandbox {
    pub fn shared() -> Self {
        Self {
            runtime: shared_runtime(),
        }
    }

    pub fn new(runtime: Arc<SandboxRuntime>) -> Self {
        Self { runtime }
    }
}

pub fn default_backend() -> Option<Arc<dyn SandboxBackend>> {
    Some(Arc::new(CatalogSandbox::shared()))
}

fn failed(status: RunStatus, message: String) -> RunResponse {
    RunResponse {
        status,
        outputs: serde_json::Value::Null,
        stdout: String::new(),
        stderr: String::new(),
        error: Some(message),
        files: Vec::new(),
        truncated: false,
        duration_ms: 0,
    }
}

#[async_trait]
impl SandboxBackend for CatalogSandbox {
    fn languages(&self) -> Vec<LanguageSupport> {
        self.runtime
            .languages()
            .into_iter()
            .map(|info| LanguageSupport {
                id: info.id.to_string(),
                installed: info.installed,
                download_bytes: info.download_bytes,
                packages: info.packages.iter().map(|p| p.to_string()).collect(),
                packages_installed: info.packages_installed,
                packages_download_bytes: info.packages_download_bytes,
            })
            .collect()
    }

    async fn run(&self, request: RunRequest) -> RunResponse {
        let Some(language) = Language::parse(&request.language) else {
            return failed(
                RunStatus::Unavailable,
                format!("Unknown language '{}'", request.language),
            );
        };
        let mut sandbox = SandboxRequest::new(language, request.code);
        sandbox.files = request
            .files
            .into_iter()
            .map(|(path, bytes)| SandboxFile::new(path, bytes.to_vec()))
            .collect();
        sandbox.limits.wall_time = request.wall_time;
        sandbox.limits.memory_bytes = request.memory_bytes;
        sandbox.limits.max_input_bytes = request.max_input_bytes;
        sandbox.packages = request.packages;
        if !request.network_hosts.is_empty() {
            sandbox.network = NetworkPolicy::AllowHosts(request.network_hosts);
        }
        let response = self.runtime.run(sandbox).await;
        RunResponse {
            status: match response.status {
                SandboxStatus::Ok => RunStatus::Ok,
                SandboxStatus::Error => RunStatus::Error,
                SandboxStatus::Timeout => RunStatus::Timeout,
                SandboxStatus::OutOfMemory => RunStatus::OutOfMemory,
                SandboxStatus::FuelExhausted => RunStatus::FuelExhausted,
                SandboxStatus::Unavailable => RunStatus::Unavailable,
            },
            outputs: response.outputs,
            stdout: response.stdout,
            stderr: response.stderr,
            error: response.error,
            truncated: response.truncated.stdout
                || response.truncated.stderr
                || response.truncated.files,
            duration_ms: response.duration_ms,
            files: response
                .files
                .into_iter()
                .map(|file| (file.path, Bytes::from(file.bytes)))
                .collect(),
        }
    }
}
