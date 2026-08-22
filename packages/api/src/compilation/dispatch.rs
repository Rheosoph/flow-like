//! Unified compilation dispatcher supporting multiple backends.
//!
//! ## Configuration
//!
//! `COMPILATION_BACKEND` selects which backend dispatches compilation jobs.
//!
//! ## Supported Backends
//!
//! There is deliberately no in-process backend. Compiling or instantiating an
//! uploaded module inside the API would run third-party code in the process
//! that holds the platform's database, mail, scheduler and bucket credentials,
//! so every deployment routes compilation to a worker. An unconfigured
//! deployment rejects publishes rather than doing the work itself.
//!
//! - **Http**: POST to a compiler worker URL (K8s pool, Docker Compose, Lambda URL)
//! - **LambdaInvoke**: AWS Lambda SDK async invocation (fire-and-forget)
//! - **Sqs**: AWS SQS queue with Lambda or ECS consumer
//! - **AzureQueue**: Azure Queue Storage queue with managed-identity workers
//! - **PubSub**: Google Cloud Pub/Sub topic with workload-identity workers.
//!   Selected by `COMPILATION_BACKEND=pubsub` (also accepted: `pub_sub`,
//!   `gcp_pubsub`) — spelled out because, unlike every other backend, the
//!   variant's snake_case name is not the value operators actually set.
//! - **Kafka**: Kafka REST Proxy for high-throughput
//! - **Redis**: Redis LPUSH/BRPOP queue (Docker Compose / K8s async)
//! - **KubernetesJob**: Spawn a dedicated K8s Job per compilation

use crate::compilation::jwt::{self, CompilerJwtParams};
use crate::routes::registry::server::all_known_targets;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::path::Path;
use flow_like_types::create_id;
use flow_like_types::dispatch::{
    CompilationJob, CompilationJobRef, CompilationStorageProvider, CompilationTarget,
    compilation_job_payload_hash,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Compilation backend type
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CompilationBackend {
    /// HTTP POST to compiler worker URL
    Http,
    /// AWS Lambda SDK async invocation (fire-and-forget)
    LambdaInvoke,
    /// AWS SQS queue for batch compilation
    Sqs,
    /// Azure Queue Storage queue for batch compilation
    AzureQueue,
    /// Google Cloud Pub/Sub topic for batch compilation
    PubSub,
    /// Apache Kafka via REST Proxy
    Kafka,
    /// Redis LPUSH/BRPOP queue
    Redis,
    /// Kubernetes Job (one job per compilation)
    KubernetesJob,
}

impl CompilationBackend {
    /// The configured backend, or `None` when this deployment has not named
    /// one. `None` is not a fallback to in-process compilation — that no
    /// longer exists — it means publish and recompile are refused.
    pub fn from_env() -> Option<Self> {
        // Explicit configuration always wins
        if let Ok(val) = std::env::var("COMPILATION_BACKEND")
            && !val.is_empty()
        {
            return Self::from_env_var("COMPILATION_BACKEND");
        }

        // Auto-detect on Lambda: prefer SQS (queue) over LambdaInvoke (direct).
        if std::env::var("AWS_LAMBDA_FUNCTION_NAME").is_ok() {
            if std::env::var("SQS_COMPILATION_QUEUE_URL").is_ok() {
                tracing::info!(
                    "Lambda detected with SQS_COMPILATION_QUEUE_URL — using Sqs compilation backend"
                );
                return Some(Self::Sqs);
            }
            if std::env::var("LAMBDA_COMPILER_FUNCTION").is_ok() {
                tracing::info!(
                    "Lambda detected with LAMBDA_COMPILER_FUNCTION — using LambdaInvoke compilation backend"
                );
                return Some(Self::LambdaInvoke);
            }
        }

        tracing::warn!(
            "No compilation backend configured — WASM package publish and recompile will be refused. Set COMPILATION_BACKEND and run a compiler worker."
        );
        None
    }

    fn from_env_var(var_name: &str) -> Option<Self> {
        match std::env::var(var_name)
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "http" | "url" => Some(Self::Http),
            "lambda_invoke" | "lambda" | "lambda_sdk" => Some(Self::LambdaInvoke),
            "sqs" | "aws_sqs" => Some(Self::Sqs),
            "azure_queue" | "azure_storage_queue" | "queue_storage" => Some(Self::AzureQueue),
            "pubsub" | "pub_sub" | "gcp_pubsub" => Some(Self::PubSub),
            "kafka" => Some(Self::Kafka),
            "redis" | "redis_queue" => Some(Self::Redis),
            "kubernetes_job" | "k8s_job" | "k8s" => Some(Self::KubernetesJob),
            "inline" | "none" | "" => {
                tracing::warn!(
                    "COMPILATION_BACKEND=inline is no longer supported — the API never compiles or instantiates WASM itself. Run a compiler worker and set COMPILATION_BACKEND=http (or a queue backend)."
                );
                None
            }
            other => {
                tracing::warn!(value = %other, "Unknown COMPILATION_BACKEND — no compilation backend configured");
                None
            }
        }
    }

    pub fn is_queue(&self) -> bool {
        matches!(
            self,
            Self::Sqs | Self::AzureQueue | Self::PubSub | Self::Kafka | Self::Redis
        )
    }

}

/// Compilation dispatch configuration loaded from environment
#[derive(Clone, Debug)]
pub struct CompilationDispatchConfig {
    pub backend: Option<CompilationBackend>,
    /// HTTP compiler URL (for Http backend)
    pub compiler_url: Option<String>,
    /// AWS Lambda function name/ARN (for LambdaInvoke backend)
    pub lambda_function_name: Option<String>,
    /// SQS queue URL (for Sqs backend)
    pub sqs_queue_url: Option<String>,
    /// Storage account hosting the work queues (for AzureQueue backend)
    pub queue_account_name: Option<String>,
    /// Azure Queue Storage compilation queue name
    pub queue_name: Option<String>,
    /// Google Cloud project that owns the Pub/Sub topic (for PubSub backend)
    pub pubsub_project: Option<String>,
    /// Pub/Sub compilation topic — either a bare topic ID or the full
    /// `projects/{project}/topics/{topic}` resource name Terraform emits
    pub pubsub_topic: Option<String>,
    /// Kafka bootstrap servers / REST proxy URL
    pub kafka_brokers: Option<String>,
    /// Kafka topic name
    pub kafka_topic: Option<String>,
    /// Redis URL (for Redis queue backend)
    pub redis_url: Option<String>,
    /// Redis queue name
    pub redis_queue_name: String,
    /// K8s namespace (for KubernetesJob backend)
    pub k8s_namespace: String,
    /// K8s compiler image
    pub k8s_compiler_image: String,
    /// API base URL for callback construction
    pub api_base_url: String,
}

impl Default for CompilationDispatchConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl CompilationDispatchConfig {
    pub fn from_env() -> Self {
        Self {
            backend: CompilationBackend::from_env(),
            compiler_url: std::env::var("COMPILER_URL").ok(),
            lambda_function_name: std::env::var("LAMBDA_COMPILER_FUNCTION").ok(),
            sqs_queue_url: std::env::var("SQS_COMPILATION_QUEUE_URL").ok(),
            queue_account_name: std::env::var("AZURE_QUEUE_STORAGE_ACCOUNT_NAME").ok(),
            queue_name: std::env::var("AZURE_QUEUE_COMPILATION").ok(),
            // Resolved exactly as `DispatchConfig` resolves it for execution:
            // `GCP_PUBSUB_PROJECT` first so an install can keep its messaging
            // plane in a project separate from the workload's own, falling back
            // to `GCP_PROJECT_ID`, which the standard deployment sets to the
            // same value. The two dispatchers must agree — resolving them
            // differently would put execution jobs in one project's topics and
            // compilation jobs in another's. Whichever wins is checked against
            // the topic resource name before publishing.
            pubsub_project: std::env::var("GCP_PUBSUB_PROJECT")
                .or_else(|_| std::env::var("GCP_PROJECT_ID"))
                .ok(),
            pubsub_topic: std::env::var("PUBSUB_COMPILATION_TOPIC").ok(),
            kafka_brokers: std::env::var("KAFKA_BROKERS").ok(),
            kafka_topic: std::env::var("KAFKA_COMPILATION_TOPIC").ok(),
            redis_url: std::env::var("REDIS_URL").ok(),
            redis_queue_name: std::env::var("REDIS_COMPILATION_QUEUE")
                .unwrap_or_else(|_| "compile:jobs".into()),
            k8s_namespace: std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "default".into()),
            k8s_compiler_image: std::env::var("K8S_COMPILER_IMAGE")
                .unwrap_or_else(|_| "flow-like-compiler:latest".into()),
            api_base_url: std::env::var("API_BASE_URL").unwrap_or_default(),
        }
    }
}

/// Response from dispatching a compilation job
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompilationDispatchResponse {
    pub job_id: String,
    pub status: String,
    pub backend: String,
}

/// Compilation dispatch errors
#[derive(Debug, thiserror::Error)]
pub enum CompilationDispatchError {
    #[error("Configuration error: {0}")]
    Configuration(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Lambda error: {0}")]
    Lambda(String),
    #[error("SQS error: {0}")]
    Sqs(String),
    #[error("Azure Queue Storage error: {0}")]
    AzureQueue(String),
    #[error("Pub/Sub error: {0}")]
    PubSub(String),
    #[error("Kafka error: {0}")]
    Kafka(String),
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("Kubernetes error: {0}")]
    Kubernetes(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("JWT error: {0}")]
    Jwt(String),
}

/// Unified compilation dispatcher (mirrors execution `Dispatcher`)
#[derive(Clone)]
pub struct CompilationDispatcher {
    config: Arc<CompilationDispatchConfig>,
    content_bucket: Arc<FlowLikeStore>,
    meta_bucket: Arc<FlowLikeStore>,
    #[cfg(feature = "lambda")]
    lambda_client: Option<aws_sdk_lambda::Client>,
    #[cfg(feature = "sqs")]
    sqs_client: Option<aws_sdk_sqs::Client>,
    #[cfg(feature = "redis")]
    redis_client: Option<redis::Client>,
}

impl CompilationDispatcher {
    pub async fn new(
        config: CompilationDispatchConfig,
        content_bucket: Arc<FlowLikeStore>,
        meta_bucket: Arc<FlowLikeStore>,
    ) -> Self {
        #[cfg(feature = "lambda")]
        let lambda_client = if config.backend == Some(CompilationBackend::LambdaInvoke) {
            let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            Some(aws_sdk_lambda::Client::new(&aws_config))
        } else {
            None
        };

        #[cfg(feature = "sqs")]
        let sqs_client = if config.backend == Some(CompilationBackend::Sqs) {
            let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            Some(aws_sdk_sqs::Client::new(&aws_config))
        } else {
            None
        };

        #[cfg(feature = "redis")]
        let redis_client = if config.backend == Some(CompilationBackend::Redis) {
            config
                .redis_url
                .as_ref()
                .and_then(|url| redis::Client::open(url.as_str()).ok())
        } else {
            None
        };

        Self {
            config: Arc::new(config),
            content_bucket,
            meta_bucket,
            #[cfg(feature = "lambda")]
            lambda_client,
            #[cfg(feature = "sqs")]
            sqs_client,
            #[cfg(feature = "redis")]
            redis_client,
        }
    }

    pub fn backend(&self) -> Option<&CompilationBackend> {
        self.config.backend.as_ref()
    }

    pub fn config(&self) -> &CompilationDispatchConfig {
        &self.config
    }

    /// Dispatch a compilation job using the configured backend
    pub async fn dispatch(
        &self,
        sub: String,
        params: DispatchParams,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        let job = build_compilation_job(
            sub,
            params,
            &self.config,
            &self.content_bucket,
            &self.meta_bucket,
        )
        .await?;
        self.dispatch_job(&job).await
    }

    /// Dispatch a pre-built compilation job
    pub async fn dispatch_job(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        match &self.config.backend {
            None => Err(CompilationDispatchError::Configuration(
                "No compilation backend configured — set COMPILATION_BACKEND and run a compiler worker. The API does not compile or instantiate WASM in-process.".into(),
            )),
            Some(CompilationBackend::Http) => self.dispatch_http(job).await,
            Some(CompilationBackend::LambdaInvoke) => self.dispatch_lambda_invoke(job).await,
            Some(CompilationBackend::Sqs) => self.dispatch_sqs(job).await,
            Some(CompilationBackend::AzureQueue) => self.dispatch_azure_queue(job).await,
            Some(CompilationBackend::PubSub) => self.dispatch_pubsub(job).await,
            Some(CompilationBackend::Kafka) => self.dispatch_kafka(job).await,
            Some(CompilationBackend::Redis) => self.dispatch_redis(job).await,
            Some(CompilationBackend::KubernetesJob) => self.dispatch_k8s_job(job).await,
        }
    }

    // ── HTTP ──────────────────────────────────────────────────────────────

    async fn dispatch_http(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        let compiler_url = self.config.compiler_url.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration("COMPILER_URL not configured".into())
        })?;

        let url = format!("{}/compile", compiler_url.trim_end_matches('/'));

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(10))
            .json(job)
            .send()
            .await
            .map_err(|e| CompilationDispatchError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(CompilationDispatchError::Network(format!(
                "Compiler returned {status}: {body}"
            )));
        }

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "dispatched".into(),
            backend: "http".into(),
        })
    }

    // ── Lambda Invoke ─────────────────────────────────────────────────────

    #[cfg(feature = "lambda")]
    async fn dispatch_lambda_invoke(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        let function_name = self.config.lambda_function_name.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration(
                "LAMBDA_COMPILER_FUNCTION not configured".into(),
            )
        })?;

        let client = self.lambda_client.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration("Lambda client not initialized".into())
        })?;

        let payload = serde_json::to_vec(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        client
            .invoke()
            .function_name(function_name)
            .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload))
            .send()
            .await
            .map_err(|e| CompilationDispatchError::Lambda(e.to_string()))?;

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "invoked".into(),
            backend: "lambda_invoke".into(),
        })
    }

    #[cfg(not(feature = "lambda"))]
    async fn dispatch_lambda_invoke(
        &self,
        _job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        Err(CompilationDispatchError::Configuration(
            "Lambda dispatch requires the 'lambda' feature. Use Http backend with Lambda Function URLs instead.".into(),
        ))
    }

    // ── SQS ───────────────────────────────────────────────────────────────

    #[cfg(feature = "sqs")]
    async fn dispatch_sqs(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        let queue_url = self.config.sqs_queue_url.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration(
                "SQS_COMPILATION_QUEUE_URL not configured".into(),
            )
        })?;

        let client = self.sqs_client.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration("SQS client not initialized".into())
        })?;

        // Stage the full payload to object storage and send only a compact
        // remote reference through SQS. The serialised CompilationJob with
        // presigned URLs for all targets easily exceeds the ~8 KB ECS
        // container-override env var limit.
        let payload_bytes = serde_json::to_vec(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        let staging_path = Path::from(format!("tmp/compilation/{}.json", job.job_id));
        self.content_bucket
            .put(&staging_path, payload_bytes)
            .await
            .map_err(|e| {
                CompilationDispatchError::Sqs(format!("Failed to stage compilation payload: {e}"))
            })?;

        let presigned_url = self
            .content_bucket
            .sign("GET", &staging_path, Duration::from_secs(86400))
            .await
            .map_err(|e| {
                CompilationDispatchError::Sqs(format!("Failed to sign staging URL: {e}"))
            })?;

        let reference = CompilationJobRef::Remote {
            remote_url: presigned_url.to_string(),
        };
        let message_body = serde_json::to_string(&reference)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        let mut req = client
            .send_message()
            .queue_url(queue_url)
            .message_body(&message_body)
            .message_group_id(&job.package_id);

        if queue_url.ends_with(".fifo") {
            req = req.message_deduplication_id(&job.job_id);
        }

        req.send()
            .await
            .map_err(|e| CompilationDispatchError::Sqs(e.to_string()))?;

        tracing::info!(
            job_id = %job.job_id,
            staging_path = %staging_path,
            "Dispatched compilation via SQS (staged to S3)"
        );

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "queued".into(),
            backend: "sqs".into(),
        })
    }

    // ── Azure Queue Storage ───────────────────────────────────────────────

    #[cfg(feature = "storage-queue")]
    async fn dispatch_azure_queue(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        use crate::storage_queue::WORKLOAD_COMPILATION;

        let account_name = self.config.queue_account_name.as_deref().ok_or_else(|| {
            CompilationDispatchError::Configuration(
                "AZURE_QUEUE_STORAGE_ACCOUNT_NAME not configured".into(),
            )
        })?;
        let queue_name = self.config.queue_name.as_deref().ok_or_else(|| {
            CompilationDispatchError::Configuration("AZURE_QUEUE_COMPILATION not configured".into())
        })?;

        // Jobs below the shared claim-check threshold keep the single-hop inline
        // path — `CompilationJobRef` is untagged, so an inline job is
        // byte-identical inside the envelope to today's message. Larger jobs are
        // staged to object storage and replaced by a remote reference instead of
        // being rejected.
        let payload_bytes = serde_json::to_vec(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;
        let payload_len = payload_bytes.len();

        if crate::storage_queue::requires_claim_check(
            payload_len,
            &job.job_id,
            &job.package_id,
            WORKLOAD_COMPILATION,
        ) {
            let staging_path = Path::from(format!("tmp/compilation/{}.json", job.job_id));
            self.content_bucket
                .put(&staging_path, payload_bytes)
                .await
                .map_err(|e| {
                    CompilationDispatchError::AzureQueue(format!(
                        "Failed to stage compilation payload: {e}"
                    ))
                })?;

            // Deliberately outlives the queue's message TTL — see
            // `CLAIM_CHECK_URL_TTL` for the margin it carries.
            let presigned_url = self
                .content_bucket
                .sign(
                    "GET",
                    &staging_path,
                    crate::storage_queue::CLAIM_CHECK_URL_TTL,
                )
                .await
                .map_err(|e| {
                    CompilationDispatchError::AzureQueue(format!("Failed to sign staging URL: {e}"))
                })?;

            let reference = CompilationJobRef::Remote {
                remote_url: presigned_url.to_string(),
            };

            crate::storage_queue::send_json(
                account_name,
                queue_name,
                &job.job_id,
                &job.package_id,
                WORKLOAD_COMPILATION,
                &reference,
            )
            .await
            .map_err(CompilationDispatchError::AzureQueue)?;

            tracing::info!(
                job_id = %job.job_id,
                staging_path = %staging_path,
                payload_bytes = payload_len,
                "Dispatched compilation via Azure Queue Storage (staged to Blob Storage)"
            );
        } else {
            crate::storage_queue::send_json(
                account_name,
                queue_name,
                &job.job_id,
                &job.package_id,
                WORKLOAD_COMPILATION,
                job,
            )
            .await
            .map_err(CompilationDispatchError::AzureQueue)?;
        }

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "queued".into(),
            backend: "azure_queue".into(),
        })
    }

    #[cfg(not(feature = "storage-queue"))]
    async fn dispatch_azure_queue(
        &self,
        _job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        Err(CompilationDispatchError::Configuration(
            "Azure Queue Storage dispatch requires the 'storage-queue' feature".into(),
        ))
    }

    #[cfg(not(feature = "sqs"))]
    async fn dispatch_sqs(
        &self,
        _job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        Err(CompilationDispatchError::Configuration(
            "SQS dispatch requires the 'sqs' feature".into(),
        ))
    }

    // ── Pub/Sub ───────────────────────────────────────────────────────────

    /// Dispatch via Google Cloud Pub/Sub using only the access token the
    /// instance metadata server mints for the workload's own service account.
    ///
    /// Pub/Sub assigns its own message ID and offers no deduplication, so — as
    /// on Azure Queue Storage — the job identity travels inside the envelope
    /// and the worker checks the resolved payload against it.
    ///
    /// There is deliberately no claim-check branch here. Queue Storage forces
    /// one because its 64 KiB ceiling sits inside the range compilation
    /// payloads occupy; Pub/Sub accepts 10 MiB, roughly 250x the largest job
    /// this API builds (nine targets x two V4-signed upload URLs). The worker
    /// still understands the untagged `CompilationJobRef::Remote` form, so
    /// adding staging later needs no worker change — but producing it today
    /// would buy a second bucket round trip and a second way to fail for a case
    /// that cannot occur.
    #[cfg(feature = "pubsub")]
    async fn dispatch_pubsub(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        let project_id = self.config.pubsub_project.as_deref().ok_or_else(|| {
            CompilationDispatchError::Configuration(
                "Neither GCP_PUBSUB_PROJECT nor GCP_PROJECT_ID is configured".into(),
            )
        })?;
        let topic = self.config.pubsub_topic.as_deref().ok_or_else(|| {
            CompilationDispatchError::Configuration(
                "PUBSUB_COMPILATION_TOPIC not configured".into(),
            )
        })?;

        let message_id = pubsub::publish_json(project_id, topic, &job.job_id, &job.package_id, job)
            .await
            .map_err(CompilationDispatchError::PubSub)?;

        tracing::info!(
            job_id = %job.job_id,
            topic = %topic,
            message_id = %message_id,
            "Dispatched compilation via Pub/Sub"
        );

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "queued".into(),
            backend: "pubsub".into(),
        })
    }

    #[cfg(not(feature = "pubsub"))]
    async fn dispatch_pubsub(
        &self,
        _job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        Err(CompilationDispatchError::Configuration(
            "Pub/Sub dispatch requires the 'pubsub' feature".into(),
        ))
    }

    // ── Kafka ─────────────────────────────────────────────────────────────

    async fn dispatch_kafka(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        let brokers = self.config.kafka_brokers.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration("KAFKA_BROKERS not configured".into())
        })?;
        let topic = self.config.kafka_topic.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration("KAFKA_COMPILATION_TOPIC not configured".into())
        })?;

        let message_body = serde_json::to_string(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        let client = reqwest::Client::new();
        let proxy_url = format!("{}/topics/{}", brokers, topic);

        let kafka_message = serde_json::json!({
            "records": [{
                "key": job.package_id,
                "value": message_body
            }]
        });

        let response = client
            .post(&proxy_url)
            .header("Content-Type", "application/vnd.kafka.json.v2+json")
            .json(&kafka_message)
            .send()
            .await
            .map_err(|e| CompilationDispatchError::Kafka(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CompilationDispatchError::Kafka(format!(
                "HTTP {status}: {text}"
            )));
        }

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "queued".into(),
            backend: "kafka".into(),
        })
    }

    // ── Redis ─────────────────────────────────────────────────────────────

    #[cfg(feature = "redis")]
    async fn dispatch_redis(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        use redis::AsyncCommands;

        let client = self.redis_client.as_ref().ok_or_else(|| {
            CompilationDispatchError::Configuration(
                "Redis client not initialized. Set REDIS_URL.".into(),
            )
        })?;

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CompilationDispatchError::Redis(e.to_string()))?;

        let message_body = serde_json::to_string(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        let queue_name = &self.config.redis_queue_name;
        conn.lpush::<_, _, ()>(queue_name, &message_body)
            .await
            .map_err(|e| CompilationDispatchError::Redis(e.to_string()))?;

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "queued".into(),
            backend: "redis".into(),
        })
    }

    #[cfg(not(feature = "redis"))]
    async fn dispatch_redis(
        &self,
        _job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        Err(CompilationDispatchError::Configuration(
            "Redis dispatch requires the 'redis' feature".into(),
        ))
    }

    // ── Kubernetes Job ────────────────────────────────────────────────────

    #[cfg(feature = "kubernetes")]
    async fn dispatch_k8s_job(
        &self,
        job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        use k8s_openapi::api::batch::v1::{Job as K8sJob, JobSpec};
        use k8s_openapi::api::core::v1::{Container, EnvVar, PodSpec, PodTemplateSpec};
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
        use kube::{Api, Client as KubeClient};

        let kube_client = KubeClient::try_default()
            .await
            .map_err(|e| CompilationDispatchError::Kubernetes(e.to_string()))?;

        let jobs_api: Api<K8sJob> = Api::namespaced(kube_client, &self.config.k8s_namespace);

        let job_payload = serde_json::to_string(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        let job_name = format!("compile-{}", &job.job_id[..8.min(job.job_id.len())]);

        let k8s_job = K8sJob {
            metadata: ObjectMeta {
                name: Some(job_name.clone()),
                labels: Some(
                    [
                        ("app".to_string(), "flow-like-compiler".to_string()),
                        ("job-id".to_string(), job.job_id.clone()),
                        ("package-id".to_string(), job.package_id.clone()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                ..Default::default()
            },
            spec: Some(JobSpec {
                ttl_seconds_after_finished: Some(300),
                backoff_limit: Some(1),
                template: PodTemplateSpec {
                    spec: Some(PodSpec {
                        restart_policy: Some("Never".to_string()),
                        containers: vec![Container {
                            name: "compiler".to_string(),
                            image: Some(self.config.k8s_compiler_image.clone()),
                            env: Some(vec![
                                EnvVar {
                                    name: "COMPILATION_JOB".to_string(),
                                    value: Some(job_payload),
                                    ..Default::default()
                                },
                                EnvVar {
                                    name: "MODE".to_string(),
                                    value: Some("single-job".to_string()),
                                    ..Default::default()
                                },
                            ]),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        };

        jobs_api
            .create(&kube::api::PostParams::default(), &k8s_job)
            .await
            .map_err(|e| CompilationDispatchError::Kubernetes(e.to_string()))?;

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "created".into(),
            backend: "kubernetes_job".into(),
        })
    }

    #[cfg(not(feature = "kubernetes"))]
    async fn dispatch_k8s_job(
        &self,
        _job: &CompilationJob,
    ) -> Result<CompilationDispatchResponse, CompilationDispatchError> {
        Err(CompilationDispatchError::Configuration(
            "Kubernetes Job dispatch requires the 'kubernetes' feature".into(),
        ))
    }
}

// ── Job construction ──────────────────────────────────────────────────────

pub struct DispatchParams {
    pub package_id: String,
    pub version: String,
    /// Storage path of the raw `.wasm` — used to generate a presigned GET URL.
    pub wasm_path: String,
    pub wasm_hash: String,
}

const WASM_COMPILED_PATH: &str = "wasm-compiled";
const URL_TTL_SECS: u64 = 3600;

pub async fn build_compilation_job(
    sub: String,
    params: DispatchParams,
    config: &CompilationDispatchConfig,
    content_bucket: &Arc<FlowLikeStore>,
    meta_bucket: &Arc<FlowLikeStore>,
) -> Result<CompilationJob, CompilationDispatchError> {
    let job_id = create_id();
    let wasm_download_provider = compilation_storage_provider(content_bucket)?;
    let upload_provider = compilation_storage_provider(meta_bucket)?;

    let wasm_download_url = content_bucket
        .sign(
            "GET",
            &Path::from(params.wasm_path.as_str()),
            Duration::from_secs(URL_TTL_SECS),
        )
        .await
        .map_err(|e| {
            CompilationDispatchError::Configuration(format!("Failed to sign .wasm GET URL: {e}"))
        })?
        .to_string();

    // Use all known targets so the external worker gets upload URLs for every
    // platform — it compiles whichever subset it supports.
    let raw_targets = all_known_targets();
    let mut targets = Vec::with_capacity(raw_targets.len());

    let base = Path::from(WASM_COMPILED_PATH)
        .child(params.package_id.as_str())
        .child(params.version.as_str());

    for t in &raw_targets {
        let cwasm_path = base.child(format!("{}.cwasm", t.platform_key));
        let checksum_path = base.child(format!("{}.cwasm.b3", t.platform_key));

        let cwasm_upload_url = meta_bucket
            .sign("PUT", &cwasm_path, Duration::from_secs(URL_TTL_SECS))
            .await
            .map_err(|e| {
                CompilationDispatchError::Configuration(format!(
                    "Failed to sign cwasm PUT URL for {}: {e}",
                    t.platform_key
                ))
            })?
            .to_string();

        let checksum_upload_url = meta_bucket
            .sign("PUT", &checksum_path, Duration::from_secs(URL_TTL_SECS))
            .await
            .map_err(|e| {
                CompilationDispatchError::Configuration(format!(
                    "Failed to sign checksum PUT URL for {}: {e}",
                    t.platform_key
                ))
            })?
            .to_string();

        targets.push(CompilationTarget {
            platform_key: t.platform_key.clone(),
            cross_triple: t.cross_triple.clone(),
            cwasm_upload_url,
            checksum_upload_url,
            upload_provider,
        });
    }

    let mut job = CompilationJob {
        job_id,
        package_id: params.package_id,
        version: params.version,
        wasm_download_url,
        wasm_download_provider,
        wasm_hash: params.wasm_hash,
        targets,
        compiler_jwt: String::new(),
    };
    let payload_hash = compilation_job_payload_hash(&job).map_err(|error| {
        CompilationDispatchError::Serialization(format!(
            "Failed to bind compilation job payload: {error}"
        ))
    })?;
    let callback_url = format!(
        "{}/api/v1/registry/compilation-callback",
        config.api_base_url.trim_end_matches('/')
    );
    job.compiler_jwt = jwt::sign(CompilerJwtParams {
        sub,
        job_id: job.job_id.clone(),
        package_id: job.package_id.clone(),
        version: job.version.clone(),
        payload_hash,
        callback_url,
        ttl_seconds: None,
    })
    .map_err(|e| CompilationDispatchError::Jwt(format!("Failed to sign compiler JWT: {e}")))?;

    Ok(job)
}

/// Declares which signing scheme authenticated the presigned URLs in a job.
///
/// The provider describes *how the URL was signed*, not where it points — the
/// worker must never infer it from the (untrusted) URL, so this mapping stays
/// keyed on the configured store.
///
/// Note that every S3-compatible endpoint (Cloudflare R2, self-hosted MinIO,
/// Ceph, …) is configured as `FlowLikeStore::AWS` and is therefore stamped
/// `AwsS3`, which makes the compiler hold its URLs to the AWS public-S3 host
/// rules. Such deployments must allowlist their endpoint host on the compiler
/// worker via `COMPILER_ALLOWED_STORAGE_HOSTS`, otherwise every job is rejected
/// as an untrusted storage host.
fn compilation_storage_provider(
    store: &FlowLikeStore,
) -> Result<CompilationStorageProvider, CompilationDispatchError> {
    match store {
        FlowLikeStore::AWS(_) => Ok(CompilationStorageProvider::AwsS3),
        FlowLikeStore::Azure(_) => Ok(CompilationStorageProvider::AzureBlob),
        FlowLikeStore::Google(_) => Ok(CompilationStorageProvider::GoogleCloudStorage),
        other => {
            let kind = match other {
                FlowLikeStore::Local(_) => "local filesystem",
                FlowLikeStore::Memory(_) => "in-memory",
                _ => "configured",
            };
            Err(CompilationDispatchError::Configuration(format!(
                "External compilation is enabled (COMPILATION_BACKEND is not `inline`), but the {kind} store cannot issue presigned URLs. Configure S3-compatible storage (AWS S3, Cloudflare R2, MinIO — all use the AWS store), Azure Blob, or Google Cloud Storage, or set COMPILATION_BACKEND=inline to compile in-process."
            )))
        }
    }
}

// ── Pub/Sub publishing ────────────────────────────────────────────────────

/// Passwordless Pub/Sub publishing for the compilation topic.
///
/// The Google counterpart to `crate::storage_queue`: the only credential this
/// path accepts is the OAuth2 token the instance metadata server mints for the
/// service account bound to the running Cloud Run revision. Service-account key
/// files, ambient `gcloud` tokens, emulator endpoints and proxies are refused
/// rather than ignored, so no long-lived secret exists anywhere that could
/// publish compilation work in this API's name.
///
/// Pub/Sub has no Rust data-plane client on the dependency line this crate
/// already links, so the single `projects.topics.publish` call is issued
/// directly against the documented REST contract — the same shape the Azure
/// `Put Message` call takes in `crate::storage_queue`.
#[cfg(feature = "pubsub")]
mod pubsub {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use flow_like_types::dispatch::CompilationJob;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::sync::OnceLock;
    use std::time::Duration;

    const PUBSUB_API_ROOT: &str = "https://pubsub.googleapis.com/v1";
    /// Narrow on purpose. Cloud Run hands out a `cloud-platform` token by
    /// default, which would open Secret Manager, GCS and Cloud SQL to anything
    /// that got hold of a copy; this token's only consumer is the Pub/Sub
    /// publish below, so the publish scope alone is all it ever needs.
    const PUBSUB_SCOPE: &str = "https://www.googleapis.com/auth/pubsub";
    /// The narrowing request, and the fallback that keeps dispatch alive if the
    /// platform refuses it.
    ///
    /// `?scopes=` alone is silently ignored — the metadata server only honours it
    /// alongside `enforce_scopes=true` (or an instance that sets the
    /// `enable-access-token-enforce-scopes` key), which is why asking for the
    /// Pub/Sub scope without it produced a full `cloud-platform` token and the
    /// illusion of a restriction. Requesting non-default scopes is in turn
    /// reported not to work under GKE Workload Identity, so a rejected narrow
    /// request retries unnarrowed against the same authority instead of costing
    /// the dispatch. Worst case is the full-scope token this path already used,
    /// with a warning naming it; best case the token cannot do anything but
    /// publish.
    const NARROWED_TOKEN_QUERY: &[(&str, &str)] =
        &[("scopes", PUBSUB_SCOPE), ("enforce_scopes", "true")];
    const UNNARROWED_TOKEN_QUERY: &[(&str, &str)] = &[];

    const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
    const METADATA_HOST: &str = "metadata.google.internal";
    /// Tried after the hostname because metadata access has to survive a
    /// revision with broken DNS — the name stops resolving long before the
    /// link-local address stops answering.
    const METADATA_IP: &str = "169.254.169.254";
    const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
    const METADATA_FLAVOR_VALUE: &str = "Google";
    /// A cached token is reused only while this much life remains, so a publish
    /// that starts just under the wire still finishes against a valid token.
    const TOKEN_REFRESH_MARGIN_SECS: i64 = 5 * 60;
    /// Ceiling on a metadata-advertised lifetime before it is trusted. Tokens
    /// live ~1h; clamping keeps an absurd value from parking a stale token in
    /// the cache for the life of the process.
    const MAX_TOKEN_LIFETIME_SECS: i64 = 24 * 60 * 60;
    const MAX_ERROR_BODY_BYTES: usize = 2_048;

    /// Envelope version understood by the queue worker. Pub/Sub mints its own
    /// `messageId` and the attribute map is unauthenticated routing metadata,
    /// so the job identity the worker checks the resolved payload against has
    /// to travel inside the body.
    const ENVELOPE_VERSION: u8 = 1;
    /// String form of [`ENVELOPE_VERSION`], because a Pub/Sub attribute map is
    /// string-to-string. Kept in step by `envelope_version_attribute_matches`.
    const ENVELOPE_VERSION_ATTRIBUTE: &str = "1";
    const WORKLOAD_COMPILATION: &str = "compilation";

    /// Pub/Sub caps one message's data plus attributes at 10 MiB. The attribute
    /// map this module sends is under 300 bytes, so nearly the whole budget is
    /// available to the body; the 64 KiB reserve is what keeps a later addition
    /// to that map from turning a dispatch that used to work into an opaque 400
    /// from the broker.
    const MAX_MESSAGE_BODY_BYTES: usize = 10 * 1024 * 1024 - 64 * 1024;

    /// Every one of these either substitutes a different credential for the
    /// workload identity or moves where the credential is fetched from, and
    /// each is rejected rather than ignored. Ignoring a set
    /// `PUBSUB_EMULATOR_HOST` would leave an operator believing jobs went to an
    /// emulator while they were published to the live topic; honouring a
    /// `GCE_METADATA_HOST` would let anything able to write this process's
    /// environment redirect the token request and steal the identity the API
    /// publishes with. `credentials/gcp_credentials.rs` deliberately *does*
    /// honour the `GCE_METADATA_*` overrides so its tokens agree with the ones
    /// `object_store` mints in the same process; this path has no such twin to
    /// stay in step with, so it refuses a redirection it cannot verify.
    ///
    /// The proxy variables are here because the publish body carries the
    /// compiler JWT and a presigned PUT URL for every target — a proxy in that
    /// path is a complete copy of the job, and therefore write access to the
    /// artifact bucket.
    const FORBIDDEN_CREDENTIAL_SETTINGS: &[&str] = &[
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_APPLICATION_CREDENTIALS_JSON",
        "GOOGLE_CREDENTIALS",
        "GOOGLE_OAUTH_ACCESS_TOKEN",
        "CLOUDSDK_AUTH_ACCESS_TOKEN",
        "GCE_METADATA_HOST",
        "GCE_METADATA_IP",
        "GCE_METADATA_ROOT",
        "PUBSUB_EMULATOR_HOST",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ];

    static PUBSUB_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    static METADATA_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    static CACHED_TOKEN: OnceLock<tokio::sync::Mutex<Option<CachedToken>>> = OnceLock::new();

    /// Deliberately holds no `Debug`: a bearer token that reaches a log line is
    /// a bearer token in the log retention window.
    struct CachedToken {
        value: String,
        expires_at_unix: i64,
    }

    /// Body of every message this API publishes to the compilation topic.
    ///
    /// `payload` is byte-for-byte the inline job, which is exactly what the
    /// untagged `CompilationJobRef::Inline` deserializes from — the same shape
    /// the Azure queue path puts on the wire, so one worker envelope parser
    /// serves both clouds.
    #[derive(Serialize)]
    struct MessageEnvelope<'a> {
        v: u8,
        job_id: &'a str,
        correlation_id: &'a str,
        workload: &'a str,
        payload: &'a CompilationJob,
    }

    /// One message per request. Batching would make a partial failure
    /// ambiguous — the response lists IDs positionally, and a dispatch that
    /// cannot say which job was accepted cannot be retried safely.
    #[derive(Serialize)]
    struct PublishRequest<'a> {
        messages: [PubsubMessage<'a>; 1],
    }

    #[derive(Serialize)]
    struct PubsubMessage<'a> {
        data: String,
        attributes: BTreeMap<&'a str, &'a str>,
    }

    #[derive(Deserialize)]
    struct PublishResponse {
        #[serde(default, rename = "messageIds")]
        message_ids: Vec<String>,
    }

    #[derive(Deserialize)]
    struct MetadataTokenResponse {
        access_token: String,
        expires_in: i64,
        #[serde(default = "bearer")]
        token_type: String,
    }

    fn bearer() -> String {
        "Bearer".to_string()
    }

    /// Publish one compilation job and return the broker-assigned message ID.
    pub(super) async fn publish_json(
        project_id: &str,
        topic: &str,
        job_id: &str,
        correlation_id: &str,
        job: &CompilationJob,
    ) -> Result<String, String> {
        reject_secret_credentials()?;
        validate_identifier("job ID", job_id)?;
        validate_identifier("correlation ID", correlation_id)?;
        let topic = qualified_topic(project_id, topic)?;

        let body = serde_json::to_vec(&MessageEnvelope {
            v: ENVELOPE_VERSION,
            job_id,
            correlation_id,
            workload: WORKLOAD_COMPILATION,
            payload: job,
        })
        .map_err(|error| format!("failed to serialize Pub/Sub message: {error}"))?;
        if body.len() > MAX_MESSAGE_BODY_BYTES {
            return Err(format!(
                "compilation message is {} bytes, above the {MAX_MESSAGE_BODY_BYTES} byte ceiling Pub/Sub accepts; the job would have to be staged to object storage and sent as a claim-check reference",
                body.len()
            ));
        }

        // The attributes duplicate what the body already states. They exist so a
        // subscription filter and the worker's dispatch switch can read the
        // routing keys without the broker or the worker parsing a megabyte of
        // JSON — never as the authority. The body is what the worker validates.
        let mut attributes = BTreeMap::new();
        attributes.insert("v", ENVELOPE_VERSION_ATTRIBUTE);
        attributes.insert("workload", WORKLOAD_COMPILATION);
        attributes.insert("job_id", job_id);
        attributes.insert("correlation_id", correlation_id);

        let request = PublishRequest {
            // `data` is a proto3 `bytes` field, so the REST contract carries it
            // base64-encoded. That also removes any question of a body byte the
            // JSON transport would have to escape or would silently mangle.
            messages: [PubsubMessage {
                data: BASE64_STANDARD.encode(&body),
                attributes,
            }],
        };

        let token = access_token().await?;
        let response = pubsub_http_client()?
            .post(format!("{PUBSUB_API_ROOT}/{topic}:publish"))
            .bearer_auth(&token)
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("failed to publish Pub/Sub message: {error}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            let body = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY_BYTES)]);
            return Err(format!(
                "Pub/Sub rejected the message on {topic} with HTTP {status}: {body}"
            ));
        }

        let published: PublishResponse = response
            .json()
            .await
            .map_err(|error| format!("invalid Pub/Sub publish response for {topic}: {error}"))?;

        // A success status with no message ID means nothing was durably stored.
        // Reporting that as `queued` would leave a package sitting in a
        // compiling state that no worker will ever move.
        published.message_ids.into_iter().next().ok_or_else(|| {
            format!("Pub/Sub accepted the publish to {topic} without returning a message ID")
        })
    }

    /// The token is cached because compilation dispatch sits on a user-facing
    /// upload request: minting per publish would put a metadata round trip in
    /// front of every package upload, and the metadata server rate-limits — a
    /// throttled refresh would fail every dispatch in flight at once.
    async fn access_token() -> Result<String, String> {
        let cached = CACHED_TOKEN.get_or_init(|| tokio::sync::Mutex::new(None));

        // The fetch happens while the lock is held. Serialising callers costs a
        // few milliseconds on a cold start and removes the thundering herd that
        // would otherwise hit the metadata server every time a token ages out.
        let mut slot = cached.lock().await;
        if let Some(token) = slot.as_ref()
            && token.expires_at_unix - unix_timestamp() > TOKEN_REFRESH_MARGIN_SECS
        {
            return Ok(token.value.clone());
        }

        let token = fetch_metadata_token().await?;
        let value = token.value.clone();
        *slot = Some(token);
        Ok(value)
    }

    async fn fetch_metadata_token() -> Result<CachedToken, String> {
        let client = metadata_http_client()?;
        let mut last_error: Option<String> = None;

        for authority in [METADATA_HOST, METADATA_IP] {
            for query in [NARROWED_TOKEN_QUERY, UNNARROWED_TOKEN_QUERY] {
                let narrowed = !query.is_empty();

                // `Metadata-Flavor: Google` is the SSRF guard, not a formality:
                // the metadata server refuses any request that omits it, so a
                // confused proxy or a redirect-following client cannot be steered
                // into reading the token on an attacker's behalf. The response
                // echo is only logged, not required — see the check below for why.
                let response = client
                    .get(format!("http://{authority}{METADATA_TOKEN_PATH}"))
                    .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
                    .query(query)
                    .send()
                    .await;

                let response = match response {
                    Ok(response) => response,
                    Err(error) => {
                        last_error = Some(format!(
                            "GCP metadata token request to {authority} failed: {error}"
                        ));
                        // The authority itself did not answer, so the fallback
                        // query would fail the same way. Move on to the next one.
                        break;
                    }
                };

                let status = response.status();
                let flavor_echoed = response
                    .headers()
                    .get(METADATA_FLAVOR_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.eq_ignore_ascii_case(METADATA_FLAVOR_VALUE));
                let body = match response.bytes().await {
                    Ok(body) => body,
                    Err(error) => {
                        last_error = Some(format!(
                            "GCP metadata response from {authority} was unreadable: {error}"
                        ));
                        break;
                    }
                };

                if !status.is_success() {
                    let body = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
                    last_error = Some(format!(
                        "GCP metadata server at {authority} returned HTTP {status}: {}",
                        String::from_utf8_lossy(body)
                    ));
                    if narrowed {
                        // The documented failure mode for a platform that will
                        // not downscope. Retry wide rather than lose the publish.
                        tracing::warn!(
                            authority,
                            %status,
                            "GCP metadata server rejected the scope-narrowed token request; retrying without scope enforcement"
                        );
                        continue;
                    }
                    break;
                }

                // Warned about, never enforced. Treating a missing echo as a
                // hijack was wrong: Google documents this header as a request
                // requirement only, `google-auth` checks it in its `ping()`
                // environment probe and on no token fetch, object_store's
                // `InstanceCredentialProvider` never checks it, and GKE has been
                // reported serving token responses without it. Failing here
                // would trade every compilation dispatch for an anti-spoofing
                // check the platform does not promise to support.
                if !flavor_echoed {
                    tracing::warn!(
                        authority,
                        "GCP metadata token response omitted {METADATA_FLAVOR_HEADER}: {METADATA_FLAVOR_VALUE}"
                    );
                }

                let payload: MetadataTokenResponse =
                    serde_json::from_slice(&body).map_err(|error| {
                        format!("invalid GCP metadata token response from {authority}: {error}")
                    })?;
                if payload.access_token.trim().is_empty() {
                    return Err(format!(
                        "GCP metadata server at {authority} returned an empty access token"
                    ));
                }
                if !payload.token_type.eq_ignore_ascii_case("Bearer") {
                    return Err(format!(
                        "GCP metadata server at {authority} returned unsupported token_type '{}'",
                        payload.token_type
                    ));
                }

                // The metadata token response carries no `scope` field, so a
                // platform that ignores the narrowing rather than rejecting it is
                // indistinguishable from one that honoured it. This warning marks
                // the case we can actually detect: the request we sent was the
                // wide one.
                if !narrowed {
                    tracing::warn!(
                        authority,
                        "publishing with an unnarrowed GCP metadata token; it carries the runtime service account's full scope"
                    );
                }

                return Ok(CachedToken {
                    expires_at_unix: unix_timestamp()
                        .saturating_add(payload.expires_in.clamp(0, MAX_TOKEN_LIFETIME_SECS)),
                    value: payload.access_token,
                });
            }
        }

        Err(last_error.unwrap_or_else(|| {
            format!(
                "no GCP credentials available: the metadata server was unreachable at {METADATA_HOST} and {METADATA_IP}. Bind a runtime service account to the Cloud Run revision, or set COMPILATION_BACKEND to a backend that does not require workload identity."
            )
        }))
    }

    fn pubsub_http_client() -> Result<&'static reqwest::Client, String> {
        if let Some(client) = PUBSUB_HTTP_CLIENT.get() {
            return Ok(client);
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("failed to construct Pub/Sub client: {error}"))?;
        let _ = PUBSUB_HTTP_CLIENT.set(client);
        PUBSUB_HTTP_CLIENT
            .get()
            .ok_or_else(|| "failed to initialize Pub/Sub client".to_string())
    }

    /// Separate from the publish client on purpose. `no_proxy` is load-bearing:
    /// reqwest honours `HTTP_PROXY` by default, so without it an ambient proxy
    /// setting — which can also arrive from a system configuration file the
    /// environment check above cannot see — would route a plaintext credential
    /// request through a third party. Redirects are refused for the same
    /// reason. `https_only` is absent by design rather than by oversight: the
    /// metadata endpoint is plain HTTP on a link-local address only the
    /// hypervisor can answer for.
    fn metadata_http_client() -> Result<&'static reqwest::Client, String> {
        if let Some(client) = METADATA_HTTP_CLIENT.get() {
            return Ok(client);
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("failed to construct GCP metadata client: {error}"))?;
        let _ = METADATA_HTTP_CLIENT.set(client);
        METADATA_HTTP_CLIENT
            .get()
            .ok_or_else(|| "failed to initialize GCP metadata client".to_string())
    }

    fn reject_secret_credentials() -> Result<(), String> {
        for variable in FORBIDDEN_CREDENTIAL_SETTINGS {
            if std::env::var(variable)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(format!(
                    "{variable} is forbidden for Pub/Sub dispatch; the API publishes with the metadata server's workload identity only"
                ));
            }
        }
        Ok(())
    }

    /// Resolve `PUBSUB_COMPILATION_TOPIC` to a fully qualified topic name and
    /// prove it belongs to the configured project.
    ///
    /// Pub/Sub addresses topics by full resource name, and the publish endpoint
    /// will happily accept one in *any* project this service account can reach.
    /// A compilation job carries a presigned PUT URL for every target plus the
    /// JWT that authorises the completion callback, so a topic pointed at a
    /// foreign project is a wholesale hand-over of write access to the artifact
    /// bucket. The resolved project is therefore not merely a convenience for
    /// expanding a bare topic ID — it is the project the configured topic must
    /// belong to, and a mismatch is refused instead of published.
    fn qualified_topic(project_id: &str, topic: &str) -> Result<String, String> {
        validate_project_id(project_id)?;

        let topic_id = match topic.strip_prefix("projects/") {
            Some(rest) => {
                let (project, topic_id) = rest.split_once("/topics/").ok_or_else(|| {
                    "PUBSUB_COMPILATION_TOPIC must be a bare topic ID or a full projects/{project}/topics/{topic} resource name".to_string()
                })?;
                if project != project_id {
                    return Err(format!(
                        "PUBSUB_COMPILATION_TOPIC names project '{project}' but GCP_PUBSUB_PROJECT/GCP_PROJECT_ID resolves to '{project_id}'; refusing to publish compilation jobs outside the configured project"
                    ));
                }
                topic_id
            }
            None => topic,
        };

        validate_topic_id(topic_id)?;
        Ok(format!("projects/{project_id}/topics/{topic_id}"))
    }

    fn validate_project_id(value: &str) -> Result<(), String> {
        let valid = (6..=30).contains(&value.len())
            && value.starts_with(|c: char| c.is_ascii_lowercase())
            && !value.ends_with('-')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err(
                "GCP_PUBSUB_PROJECT/GCP_PROJECT_ID must be 6-30 characters of lowercase letters, digits and hyphens, starting with a letter"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Pub/Sub's own topic-ID rules, enforced here rather than left to the
    /// broker: a value containing `/` or `:` would not be rejected as a bad
    /// name, it would silently retarget the request at a different resource
    /// path on the same API.
    fn validate_topic_id(value: &str) -> Result<(), String> {
        let valid = (3..=255).contains(&value.len())
            && value.starts_with(|c: char| c.is_ascii_alphabetic())
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'+' | b'%')
            })
            && !value
                .get(..4)
                .is_some_and(|head| head.eq_ignore_ascii_case("goog"));
        if !valid {
            return Err(format!(
                "'{value}' is not a valid Pub/Sub topic ID (3-255 characters, starting with a letter, no reserved 'goog' prefix)"
            ));
        }
        Ok(())
    }

    /// These identifiers become Pub/Sub attribute values as well as envelope
    /// fields. A control character there is not merely ugly: it makes the
    /// message unparseable for the worker that would otherwise dead-letter it,
    /// so it is rejected at the producer where the job can still fail cleanly.
    fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(format!("Pub/Sub message {kind} is invalid"));
        }
        Ok(())
    }

    fn unix_timestamp() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn envelope_version_attribute_matches() {
            assert_eq!(ENVELOPE_VERSION.to_string(), ENVELOPE_VERSION_ATTRIBUTE);
        }

        #[test]
        fn bare_topic_ids_are_qualified_with_the_configured_project() {
            assert_eq!(
                qualified_topic("flow-like-dev", "flow-like-compilation"),
                Ok("projects/flow-like-dev/topics/flow-like-compilation".to_string())
            );
        }

        #[test]
        fn fully_qualified_topics_survive_unchanged() {
            assert_eq!(
                qualified_topic(
                    "flow-like-dev",
                    "projects/flow-like-dev/topics/flow-like-compilation"
                ),
                Ok("projects/flow-like-dev/topics/flow-like-compilation".to_string())
            );
        }

        #[test]
        fn topics_in_another_project_are_refused() {
            assert!(
                qualified_topic("flow-like-dev", "projects/attacker-proj/topics/compilation")
                    .is_err()
            );
        }

        #[test]
        fn topic_ids_cannot_smuggle_a_different_resource_path() {
            assert!(qualified_topic("flow-like-dev", "compilation:publish").is_err());
            assert!(qualified_topic("flow-like-dev", "../subscriptions/x").is_err());
            assert!(qualified_topic("flow-like-dev", "goog-reserved").is_err());
            assert!(qualified_topic("flow-like-dev", "1compilation").is_err());
            assert!(qualified_topic("flow-like-dev", "ab").is_err());
        }

        #[test]
        fn project_ids_are_restricted_to_the_documented_charset() {
            assert!(validate_project_id("flow-like-dev").is_ok());
            assert!(validate_project_id("Flow-Like-Dev").is_err());
            assert!(validate_project_id("short").is_err());
            assert!(validate_project_id("flow-like-").is_err());
            assert!(validate_project_id("flow/like/dev").is_err());
        }

        #[test]
        fn identifiers_with_control_characters_never_reach_an_attribute() {
            assert!(validate_identifier("job ID", "job-1").is_ok());
            assert!(validate_identifier("job ID", "job\n1").is_err());
            assert!(validate_identifier("job ID", "").is_err());
            assert!(validate_identifier("job ID", &"a".repeat(129)).is_err());
        }
    }
}
