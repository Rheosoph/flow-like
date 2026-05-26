//! Kubernetes configuration for job execution

use serde::{Deserialize, Serialize};

/// Configuration for Kubernetes job execution
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KubernetesConfig {
    /// Kubernetes namespace for executor jobs
    pub namespace: String,
    /// Docker image for the executor
    pub executor_image: String,
    /// Runtime class for isolation (e.g., "kata" for Kata Containers)
    pub runtime_class: Option<String>,
    /// Job timeout in seconds
    pub job_timeout_seconds: u64,
    /// Maximum retries for failed jobs
    pub job_max_retries: u32,
    /// URL for warm executor pool (optional)
    pub executor_pool_url: Option<String>,
    /// Memory request for executor pods
    pub memory_request: String,
    /// Memory limit for executor pods
    pub memory_limit: String,
    /// CPU request for executor pods
    pub cpu_request: String,
    /// CPU limit for executor pods
    pub cpu_limit: String,
}

impl Default for KubernetesConfig {
    fn default() -> Self {
        Self {
            namespace: "flow-like".to_string(),
            executor_image: "ghcr.io/rheosoph/flow-like-k8s-executor:latest".to_string(),
            runtime_class: None,
            job_timeout_seconds: 3600,
            job_max_retries: 3,
            executor_pool_url: None,
            memory_request: "256Mi".to_string(),
            memory_limit: "2Gi".to_string(),
            cpu_request: "100m".to_string(),
            cpu_limit: "2".to_string(),
        }
    }
}

impl KubernetesConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        Self {
            namespace: std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "flow-like".to_string()),
            executor_image: std::env::var("K8S_EXECUTOR_IMAGE")
                .unwrap_or_else(|_| "ghcr.io/rheosoph/flow-like-k8s-executor:latest".to_string()),
            runtime_class: std::env::var("K8S_RUNTIME_CLASS").ok(),
            job_timeout_seconds: std::env::var("K8S_JOB_TIMEOUT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3600),
            job_max_retries: std::env::var("K8S_JOB_MAX_RETRIES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            executor_pool_url: std::env::var("K8S_EXECUTOR_POOL_URL").ok(),
            memory_request: std::env::var("K8S_MEMORY_REQUEST")
                .unwrap_or_else(|_| "256Mi".to_string()),
            memory_limit: std::env::var("K8S_MEMORY_LIMIT").unwrap_or_else(|_| "2Gi".to_string()),
            cpu_request: std::env::var("K8S_CPU_REQUEST").unwrap_or_else(|_| "100m".to_string()),
            cpu_limit: std::env::var("K8S_CPU_LIMIT").unwrap_or_else(|_| "2".to_string()),
        }
    }
}
