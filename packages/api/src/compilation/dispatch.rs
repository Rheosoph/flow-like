//! Unified compilation dispatcher supporting multiple backends.
//!
//! ## Configuration
//!
//! `COMPILATION_BACKEND` selects which backend dispatches compilation jobs.
//!
//! ## Supported Backends
//!
//! - **Inline** (default): Compile in-process (API process does AOT compilation)
//! - **Http**: POST to a compiler worker URL (K8s pool, Docker Compose, Lambda URL)
//! - **LambdaInvoke**: AWS Lambda SDK async invocation (fire-and-forget)
//! - **Sqs**: AWS SQS queue with Lambda or ECS consumer
//! - **Kafka**: Kafka REST Proxy for high-throughput
//! - **Redis**: Redis LPUSH/BRPOP queue (Docker Compose / K8s async)
//! - **KubernetesJob**: Spawn a dedicated K8s Job per compilation

use crate::compilation::jwt::{self, CompilerJwtParams};
use crate::routes::registry::server::{TargetSpec, all_known_targets, compilation_targets};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::path::Path;
use flow_like_types::create_id;
use flow_like_types::dispatch::{CompilationJob, CompilationTarget};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Compilation backend type
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CompilationBackend {
    /// In-process AOT compilation (no external worker)
    #[default]
    Inline,
    /// HTTP POST to compiler worker URL
    Http,
    /// AWS Lambda SDK async invocation (fire-and-forget)
    LambdaInvoke,
    /// AWS SQS queue for batch compilation
    Sqs,
    /// Apache Kafka via REST Proxy
    Kafka,
    /// Redis LPUSH/BRPOP queue
    Redis,
    /// Kubernetes Job (one job per compilation)
    KubernetesJob,
}

impl CompilationBackend {
    pub fn from_env() -> Self {
        // Explicit configuration always wins
        if let Ok(val) = std::env::var("COMPILATION_BACKEND") {
            if !val.is_empty() {
                return Self::from_env_var("COMPILATION_BACKEND");
            }
        }

        // Auto-detect on Lambda: prefer SQS (queue) over LambdaInvoke (direct)
        // over Inline — inline AOT compilation inside the API Lambda is almost
        // never desired (limited memory/time).
        if std::env::var("AWS_LAMBDA_FUNCTION_NAME").is_ok() {
            if std::env::var("SQS_COMPILATION_QUEUE_URL").is_ok() {
                tracing::info!("Lambda detected with SQS_COMPILATION_QUEUE_URL — using Sqs compilation backend");
                return Self::Sqs;
            }
            if std::env::var("LAMBDA_COMPILER_FUNCTION").is_ok() {
                tracing::info!("Lambda detected with LAMBDA_COMPILER_FUNCTION — using LambdaInvoke compilation backend");
                return Self::LambdaInvoke;
            }
            tracing::warn!(
                "Running on Lambda without COMPILATION_BACKEND, SQS_COMPILATION_QUEUE_URL, or LAMBDA_COMPILER_FUNCTION — falling back to Inline (not recommended)"
            );
        }

        Self::Inline
    }

    fn from_env_var(var_name: &str) -> Self {
        match std::env::var(var_name)
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "http" | "url" => Self::Http,
            "lambda_invoke" | "lambda" | "lambda_sdk" => Self::LambdaInvoke,
            "sqs" | "aws_sqs" => Self::Sqs,
            "kafka" => Self::Kafka,
            "redis" | "redis_queue" => Self::Redis,
            "kubernetes_job" | "k8s_job" | "k8s" => Self::KubernetesJob,
            "inline" | "" => Self::Inline,
            other => {
                tracing::warn!(value = %other, "Unknown COMPILATION_BACKEND, falling back to inline");
                Self::Inline
            }
        }
    }

    pub fn is_queue(&self) -> bool {
        matches!(self, Self::Sqs | Self::Kafka | Self::Redis)
    }

    pub fn is_external(&self) -> bool {
        !matches!(self, Self::Inline)
    }
}

/// Compilation dispatch configuration loaded from environment
#[derive(Clone, Debug)]
pub struct CompilationDispatchConfig {
    pub backend: CompilationBackend,
    /// HTTP compiler URL (for Http backend)
    pub compiler_url: Option<String>,
    /// AWS Lambda function name/ARN (for LambdaInvoke backend)
    pub lambda_function_name: Option<String>,
    /// SQS queue URL (for Sqs backend)
    pub sqs_queue_url: Option<String>,
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
        let lambda_client = if config.backend == CompilationBackend::LambdaInvoke {
            let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            Some(aws_sdk_lambda::Client::new(&aws_config))
        } else {
            None
        };

        #[cfg(feature = "sqs")]
        let sqs_client = if config.backend == CompilationBackend::Sqs {
            let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            Some(aws_sdk_sqs::Client::new(&aws_config))
        } else {
            None
        };

        #[cfg(feature = "redis")]
        let redis_client = if config.backend == CompilationBackend::Redis {
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

    pub fn backend(&self) -> &CompilationBackend {
        &self.config.backend
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
            CompilationBackend::Inline => Err(CompilationDispatchError::Configuration(
                "Inline backend does not dispatch — caller should compile in-process".into(),
            )),
            CompilationBackend::Http => self.dispatch_http(job).await,
            CompilationBackend::LambdaInvoke => self.dispatch_lambda_invoke(job).await,
            CompilationBackend::Sqs => self.dispatch_sqs(job).await,
            CompilationBackend::Kafka => self.dispatch_kafka(job).await,
            CompilationBackend::Redis => self.dispatch_redis(job).await,
            CompilationBackend::KubernetesJob => self.dispatch_k8s_job(job).await,
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

        let message_body = serde_json::to_string(job)
            .map_err(|e| CompilationDispatchError::Serialization(e.to_string()))?;

        let mut req = client
            .send_message()
            .queue_url(queue_url)
            .message_body(&message_body)
            // MessageGroupId enables fair queueing per package on standard queues
            // and ordering per group on FIFO queues.
            .message_group_id(&job.package_id);

        // MessageDeduplicationId is only valid for FIFO queues
        if queue_url.ends_with(".fifo") {
            req = req.message_deduplication_id(&job.job_id);
        }

        req.send()
            .await
            .map_err(|e| CompilationDispatchError::Sqs(e.to_string()))?;

        Ok(CompilationDispatchResponse {
            job_id: job.job_id.clone(),
            status: "queued".into(),
            backend: "sqs".into(),
        })
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

    let callback_url = format!(
        "{}/registry/compilation-callback",
        config.api_base_url.trim_end_matches('/')
    );

    let compiler_jwt = jwt::sign(CompilerJwtParams {
        sub: sub.clone(),
        job_id: job_id.clone(),
        package_id: params.package_id.clone(),
        version: params.version.clone(),
        callback_url,
        ttl_seconds: None,
    })
    .map_err(|e| CompilationDispatchError::Jwt(format!("Failed to sign compiler JWT: {e}")))?;

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
        });
    }

    Ok(CompilationJob {
        job_id,
        package_id: params.package_id,
        version: params.version,
        wasm_download_url,
        wasm_hash: params.wasm_hash,
        targets,
        compiler_jwt,
    })
}
