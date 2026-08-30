//! Kubernetes job dispatch for workflow execution

use super::KubernetesConfig;
use flow_like_types::create_id;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PodSpec, PodTemplateSpec, ResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::{
    Api, Client,
    api::{DeleteParams, PostParams},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Job execution mode
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobMode {
    /// Run in an isolated Kubernetes Job (started and stopped for this execution)
    #[default]
    Isolated,
    /// Run in a warm executor pool (faster startup, shared resources)
    Standard,
}

impl From<&str> for JobMode {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "standard" | "warm" | "pool" => JobMode::Standard,
            _ => JobMode::Isolated,
        }
    }
}

/// Request to submit a job for execution
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitJobRequest {
    /// Run ID (unique per execution, used in JWT)
    pub run_id: String,
    /// Application ID
    pub app_id: String,
    /// Board ID to execute
    pub board_id: String,
    /// Optional board version
    pub version: Option<String>,
    /// Optional event ID (for event-triggered executions)
    pub event_id: Option<String>,
    /// Input payload for the execution
    pub payload: Option<serde_json::Value>,
    /// Execution mode (isolated or standard/warm)
    pub mode: JobMode,
    /// User ID initiating the execution
    pub user_id: String,
    /// Execution context containing credentials, JWT, and callback URL
    pub execution_context: ExecutionContext,
}

/// Execution context passed to the executor
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// Serialized RuntimeCredentials for storage access
    pub credentials_json: String,
    /// JWT for authenticating progress callbacks
    pub jwt: String,
    /// Base URL of the API for callbacks (e.g., "https://api.flow-like.io")
    pub callback_url: String,
    /// Channel grant the run waits on client replies through
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<flow_like_types::channel::ChannelGrant>,
}

/// Response from job submission
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitJobResponse {
    /// Unique job ID
    pub job_id: String,
    /// Current job status
    pub status: String,
}

/// Job status information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStatus {
    /// Job ID
    pub job_id: String,
    /// Current status (queued, running, completed, failed)
    pub status: String,
    /// When the job started
    pub started_at: Option<String>,
    /// When the job completed
    pub completed_at: Option<String>,
    /// Error message if failed
    pub error: Option<String>,
}

/// Kubernetes job dispatcher
#[derive(Clone)]
pub struct JobDispatcher {
    config: Arc<KubernetesConfig>,
}

impl JobDispatcher {
    /// Create a new job dispatcher with the given configuration
    pub fn new(config: KubernetesConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Submit a job for execution
    pub async fn submit(
        &self,
        request: SubmitJobRequest,
    ) -> Result<SubmitJobResponse, DispatchError> {
        let job_id = create_id();

        // Dispatch based on mode
        match request.mode {
            JobMode::Standard => {
                self.dispatch_to_pool(&job_id, &request).await?;
            }
            JobMode::Isolated => {
                self.create_k8s_job(&job_id, &request).await?;
            }
        }

        tracing::info!(
            job_id = %job_id,
            app_id = %request.app_id,
            board_id = %request.board_id,
            mode = ?request.mode,
            "Job submitted"
        );

        Ok(SubmitJobResponse {
            job_id,
            status: "queued".to_string(),
        })
    }

    /// Dispatch job to warm executor pool
    async fn dispatch_to_pool(
        &self,
        job_id: &str,
        request: &SubmitJobRequest,
    ) -> Result<(), DispatchError> {
        let pool_url = self.config.executor_pool_url.clone().ok_or_else(|| {
            DispatchError::Configuration("executor_pool_url not configured".to_string())
        })?;

        let body = serde_json::json!({
            "job_id": job_id,
            "run_id": request.run_id,
            "app_id": request.app_id,
            "board_id": request.board_id,
            "version": request.version,
            "event_id": request.event_id,
            "payload": request.payload,
            "user_id": request.user_id,
            "credentials": request.execution_context.credentials_json,
            "jwt": request.execution_context.jwt,
            "callback_url": request.execution_context.callback_url,
            "channel": request.execution_context.channel,
        });

        let client = reqwest::Client::new();
        client
            .post(format!("{}/execute", pool_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| DispatchError::Network(e.to_string()))?
            .error_for_status()
            .map_err(|e| DispatchError::Network(e.to_string()))?;

        Ok(())
    }

    /// Create an isolated Kubernetes Job
    async fn create_k8s_job(
        &self,
        job_id: &str,
        request: &SubmitJobRequest,
    ) -> Result<(), DispatchError> {
        let client = Client::try_default()
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        let jobs: Api<Job> = Api::namespaced(client, &self.config.namespace);
        let job = self.build_job_spec(job_id, request);

        jobs.create(&PostParams::default(), &job)
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        Ok(())
    }

    /// Build the Kubernetes Job specification
    fn build_job_spec(&self, job_id: &str, request: &SubmitJobRequest) -> Job {
        let mut labels = BTreeMap::new();
        labels.insert("app".to_string(), "flow-like-executor".to_string());
        labels.insert("job-id".to_string(), job_id.to_string());
        labels.insert("app-id".to_string(), request.app_id.clone());

        let mut annotations = BTreeMap::new();
        annotations.insert("flow-like.io/run-id".to_string(), request.run_id.clone());
        annotations.insert(
            "flow-like.io/board-id".to_string(),
            request.board_id.clone(),
        );
        annotations.insert("flow-like.io/user-id".to_string(), request.user_id.clone());
        if let Some(event_id) = &request.event_id {
            annotations.insert("flow-like.io/event-id".to_string(), event_id.clone());
        }

        let mut env_vars = vec![
            EnvVar {
                name: "RUN_ID".to_string(),
                value: Some(request.run_id.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "JOB_ID".to_string(),
                value: Some(job_id.to_string()),
                ..Default::default()
            },
            EnvVar {
                name: "APP_ID".to_string(),
                value: Some(request.app_id.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "BOARD_ID".to_string(),
                value: Some(request.board_id.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "USER_ID".to_string(),
                value: Some(request.user_id.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "FLOW_LIKE_CREDENTIALS".to_string(),
                value: Some(request.execution_context.credentials_json.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "FLOW_LIKE_JWT".to_string(),
                value: Some(request.execution_context.jwt.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "FLOW_LIKE_CALLBACK_URL".to_string(),
                value: Some(request.execution_context.callback_url.clone()),
                ..Default::default()
            },
            EnvVar {
                name: "API_BASE_URL".to_string(),
                value: Some(request.execution_context.callback_url.clone()),
                ..Default::default()
            },
        ];

        if let Some(event_id) = &request.event_id {
            env_vars.push(EnvVar {
                name: "EVENT_ID".to_string(),
                value: Some(event_id.clone()),
                ..Default::default()
            });
        }

        if let Some(payload) = &request.payload {
            env_vars.push(EnvVar {
                name: "PAYLOAD".to_string(),
                value: Some(payload.to_string()),
                ..Default::default()
            });
        }

        if let Some(channel) = &request.execution_context.channel
            && let Ok(grant) = serde_json::to_string(channel)
        {
            env_vars.push(EnvVar {
                name: "FLOW_LIKE_CHANNEL".to_string(),
                value: Some(grant),
                ..Default::default()
            });
        }

        let mut resource_requests = BTreeMap::new();
        resource_requests.insert(
            "memory".to_string(),
            Quantity(self.config.memory_request.clone()),
        );
        resource_requests.insert("cpu".to_string(), Quantity(self.config.cpu_request.clone()));

        let mut resource_limits = BTreeMap::new();
        resource_limits.insert(
            "memory".to_string(),
            Quantity(self.config.memory_limit.clone()),
        );
        resource_limits.insert("cpu".to_string(), Quantity(self.config.cpu_limit.clone()));

        let container = Container {
            name: "executor".to_string(),
            image: Some(self.config.executor_image.clone()),
            env: Some(env_vars),
            resources: Some(ResourceRequirements {
                requests: Some(resource_requests),
                limits: Some(resource_limits),
                ..Default::default()
            }),
            ..Default::default()
        };

        let mut pod_spec = PodSpec {
            containers: vec![container],
            restart_policy: Some("Never".to_string()),
            ..Default::default()
        };

        // Add runtime class if configured (e.g., for Kata Containers)
        if let Some(runtime_class) = &self.config.runtime_class {
            pod_spec.runtime_class_name = Some(runtime_class.clone());
        }

        Job {
            metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                name: Some(format!("flow-like-{}", job_id)),
                namespace: Some(self.config.namespace.clone()),
                labels: Some(labels),
                annotations: Some(annotations),
                ..Default::default()
            },
            spec: Some(k8s_openapi::api::batch::v1::JobSpec {
                template: PodTemplateSpec {
                    spec: Some(pod_spec),
                    ..Default::default()
                },
                backoff_limit: Some(self.config.job_max_retries as i32),
                active_deadline_seconds: Some(self.config.job_timeout_seconds as i64),
                ttl_seconds_after_finished: Some(3600), // Cleanup after 1 hour
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Get the status of a job
    pub async fn get_status(&self, job_id: &str) -> Result<JobStatus, DispatchError> {
        let client = Client::try_default()
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        let jobs: Api<Job> = Api::namespaced(client, &self.config.namespace);

        let job_name = format!("flow-like-{}", job_id);
        let job = jobs
            .get(&job_name)
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        let status = job.status.as_ref();
        let (job_status, error) = match status {
            Some(s) if s.succeeded.unwrap_or(0) > 0 => ("completed".to_string(), None),
            Some(s) if s.failed.unwrap_or(0) > 0 => (
                "failed".to_string(),
                Some("Job execution failed".to_string()),
            ),
            Some(s) if s.active.unwrap_or(0) > 0 => ("running".to_string(), None),
            _ => ("pending".to_string(), None),
        };

        let started_at = status
            .and_then(|s| s.start_time.as_ref())
            .map(|t| t.0.to_rfc3339());

        let completed_at = status
            .and_then(|s| s.completion_time.as_ref())
            .map(|t| t.0.to_rfc3339());

        Ok(JobStatus {
            job_id: job_id.to_string(),
            status: job_status,
            started_at,
            completed_at,
            error,
        })
    }

    /// Cancel a running job
    pub async fn cancel(&self, job_id: &str) -> Result<(), DispatchError> {
        let client = Client::try_default()
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        let jobs: Api<Job> = Api::namespaced(client, &self.config.namespace);

        let job_name = format!("flow-like-{}", job_id);
        jobs.delete(&job_name, &DeleteParams::default())
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        tracing::info!(job_id = %job_id, "Job cancelled");
        Ok(())
    }
}

/// Errors that can occur during job dispatch
#[derive(Debug)]
pub enum DispatchError {
    /// Error generating presigned URLs
    Presign(String),
    /// Error serializing job data
    Serialization(String),
    /// Network error (upload, pool dispatch)
    Network(String),
    /// Kubernetes API error
    Kubernetes(String),
    /// Configuration error
    Configuration(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Presign(msg) => write!(f, "Presign error: {}", msg),
            DispatchError::Serialization(msg) => write!(f, "Serialization error: {}", msg),
            DispatchError::Network(msg) => write!(f, "Network error: {}", msg),
            DispatchError::Kubernetes(msg) => write!(f, "Kubernetes error: {}", msg),
            DispatchError::Configuration(msg) => write!(f, "Configuration error: {}", msg),
        }
    }
}

impl std::error::Error for DispatchError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_jobs_receive_the_api_proxy_base_url() {
        let dispatcher = JobDispatcher::new(KubernetesConfig::default());
        let request = SubmitJobRequest {
            run_id: "run-1".to_string(),
            app_id: "app-1".to_string(),
            board_id: "board-1".to_string(),
            version: None,
            event_id: None,
            payload: None,
            mode: JobMode::Isolated,
            user_id: "user-1".to_string(),
            execution_context: ExecutionContext {
                credentials_json: "{}".to_string(),
                jwt: "token".to_string(),
                callback_url: "https://api.example.test".to_string(),
                channel: Some(flow_like_types::channel::ChannelGrant {
                    channel_id: "run-1".to_string(),
                    expires_at: 1,
                    executor: flow_like_types::channel::ChannelExecutorGrant::Http {},
                    client: flow_like_types::channel::ChannelHandle {
                        channel_id: "run-1".to_string(),
                        request_id: None,
                        expires_at: 1,
                        transport: flow_like_types::channel::ChannelClientDescriptor::Http {
                            push_url: "https://api.example.test/api/v1/channels/run-1/push"
                                .to_string(),
                            token: "t".to_string(),
                        },
                        fallback: None,
                    },
                }),
            },
        };

        let job = dispatcher.build_job_spec("job-1", &request);
        let env = job
            .spec
            .and_then(|spec| spec.template.spec)
            .and_then(|pod| pod.containers.into_iter().next())
            .and_then(|container| container.env)
            .expect("executor environment");
        let api_base_url = env
            .iter()
            .find(|variable| variable.name == "API_BASE_URL")
            .and_then(|variable| variable.value.as_deref());

        assert_eq!(api_base_url, Some("https://api.example.test"));

        let channel = env
            .iter()
            .find(|variable| variable.name == "FLOW_LIKE_CHANNEL")
            .and_then(|variable| variable.value.as_deref())
            .expect("channel grant env");
        let grant: flow_like_types::channel::ChannelGrant = serde_json::from_str(channel).unwrap();
        assert_eq!(grant.channel_id, "run-1");
    }
}
