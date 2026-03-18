//! Unified job dispatcher supporting multiple execution backends.
//!
//! ## Configuration
//!
//! Two environment variables control dispatch behavior:
//!
//! - **`EXECUTION_BACKEND`**: Used for `/invoke` (streaming/sync) endpoints
//! - **`ASYNC_EXECUTION_BACKEND`**: Used for `/invoke/async` endpoints
//!
//! Both support the same backend options, allowing different backends for
//! realtime streaming vs background batch processing.
//!
//! ## Supported Backends
//!
//! - **HTTP**: Direct HTTP call to executor endpoint. Works with ALL platforms:
//!   - Kubernetes executor pool
//!   - Docker Compose runtime
//!   - AWS Lambda (via Function URLs)
//!   - Azure Functions
//!   - GCP Cloud Functions
//! - **LambdaInvoke**: AWS Lambda SDK invocation (async batch optimization, fire-and-forget)
//! - **LambdaStream**: AWS Lambda SDK streaming invocation (returns streaming response)
//! - **KubernetesJob**: Native K8s Job creation for isolated executions
//! - **Sqs**: AWS SQS queue for batch processing with Lambda consumer
//! - **SqsEventBridge**: SQS → EventBridge Pipe → ECS RunTask (payload staged to storage)
//! - **Kafka**: Apache Kafka queue for high-throughput batch processing
//! - **Redis**: Redis queue for Docker Compose / Kubernetes async dispatch
//!
//! ## Isolation & Security Model
//!
//! ### Lambda (HTTP / LambdaInvoke / LambdaStream)
//!
//! AWS Lambda provides **strong isolation** via AWS Firecracker microVMs:
//! - Each execution runs in its own microVM with hardware-level isolation
//! - Memory is wiped between invocations from different tenants
//! - No shared filesystem between executions
//! - Cold starts create fresh environments; warm starts reuse the same microVM
//!   for the **same function** only (not shared across tenants)
//!
//! **Best for**: Multi-tenant workloads requiring strong isolation guarantees.
//!
//! ### Kubernetes Warm Pool (HTTP → K8s Deployment)
//!
//! A pool of long-running executor pods handles requests:
//! - **Process-level isolation**: Each request runs in the same pod but can use
//!   separate processes or containers within the pod
//! - **Shared resources**: Pods may handle multiple requests over their lifetime
//! - **Faster response**: No cold start - pods are already running
//! - **Cost efficient**: Fewer pod creations, better resource utilization
//!
//! **Security consideration**: Requests from different users may run on the same
//! pod. Ensure the executor cleans up state between requests. Suitable when:
//! - Tenants are trusted (same organization)
//! - Execution code is sandboxed (e.g., WASM, containers within pods)
//! - Performance is prioritized over strict isolation
//!
//! **Best for**: Internal/trusted workloads, low-latency requirements.
//!
//! ### Kubernetes Isolated Job (KubernetesJob)
//!
//! Each execution creates a dedicated Kubernetes Job:
//! - **Pod-level isolation**: Fresh pod for every execution
//! - **Resource guarantees**: Dedicated CPU/memory per job
//! - **Clean environment**: No state leakage between executions
//! - **Network policies**: Can apply per-job network restrictions
//! - **Slower startup**: Pod scheduling + image pull overhead
//!
//! **Best for**: Untrusted code execution, strict compliance requirements,
//! resource-intensive workloads needing guaranteed resources.
//!
//! ### Docker Compose (HTTP)
//!
//! For local development and small deployments:
//! - **Container-level isolation**: Each executor is a separate container
//! - **Shared host resources**: Containers share the Docker host
//! - **Simpler setup**: No orchestration complexity
//!
//! **Best for**: Development, testing, small-scale deployments.
//!
//! ## Choosing a Backend
//!
//! | Requirement | Recommended Backend |
//! |-------------|---------------------|
//! | Multi-tenant SaaS | Lambda (strongest isolation) |
//! | Low latency | HTTP → Warm Pool (K8s/Lambda) |
//! | Untrusted code | KubernetesJob or Lambda |
//! | Batch processing | SQS, Kafka, or Redis (decoupled, retry built-in) |
//! | Long-running batch | SqsEventBridge (ECS with no timeout limits) |
//! | High-throughput batch | Kafka (millions/sec, partitioned) |
//! | Streaming response | HTTP or LambdaStream |
//! | Cost optimization | HTTP → Warm Pool |
//! | Compliance/audit | KubernetesJob (per-job logging) |
//!
//! ## Typical Configuration
//!
//! ```bash
//! # Streaming uses HTTP for realtime SSE response
//! EXECUTION_BACKEND=http
//!
//! # Async uses Redis queue for background processing
//! ASYNC_EXECUTION_BACKEND=redis
//! ```

use flow_like_storage::Path as StorePath;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::create_id;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;

/// A streaming byte chunk result
pub type StreamChunk = Result<bytes::Bytes, DispatchError>;

/// A boxed stream of byte chunks
pub type ByteStream = Pin<Box<dyn futures::Stream<Item = StreamChunk> + Send>>;

/// Execution backend type
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBackend {
    /// HTTP endpoint - works with ALL platforms:
    /// K8s pool, Docker Compose, Lambda Function URLs, Azure Functions, GCP Cloud Functions
    #[default]
    Http,
    /// AWS Lambda SDK invocation (async batch optimization, fire-and-forget)
    LambdaInvoke,
    /// AWS Lambda SDK streaming invocation (returns streaming response from private Lambdas)
    LambdaStream,
    /// Kubernetes Job (isolated, one job per execution)
    KubernetesJob,
    /// AWS SQS queue for batch processing (Lambda consumer with callback)
    Sqs,
    /// Apache Kafka for high-throughput batch processing
    Kafka,
    /// Redis queue for Docker Compose / Kubernetes async dispatch
    Redis,
    /// SQS → EventBridge Pipe → ECS RunTask for long-running executions.
    /// Stages full payload to object storage (avoids 8 KB ECS env var limit)
    /// and sends only a presigned URL reference via SQS.
    SqsEventBridge,
}

impl ExecutionBackend {
    pub fn from_env() -> Self {
        Self::from_env_var("EXECUTION_BACKEND")
    }

    pub fn async_from_env() -> Self {
        Self::from_env_var("ASYNC_EXECUTION_BACKEND")
    }

    fn from_env_var(var_name: &str) -> Self {
        match std::env::var(var_name)
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "lambda_invoke" | "lambda_sdk" => Self::LambdaInvoke,
            "lambda_stream" | "lambda_streaming" => Self::LambdaStream,
            "kubernetes_job" | "k8s_job" | "isolated" => Self::KubernetesJob,
            "sqs" | "aws_sqs" => Self::Sqs,
            "kafka" => Self::Kafka,
            "redis" | "redis_queue" => Self::Redis,
            "sqs_event_bridge" | "sqs_ecs" | "ecs" => Self::SqsEventBridge,
            _ => Self::Http,
        }
    }

    pub fn is_lambda(&self) -> bool {
        matches!(self, Self::LambdaInvoke | Self::LambdaStream)
    }

    pub fn is_queue(&self) -> bool {
        matches!(
            self,
            Self::Sqs | Self::SqsEventBridge | Self::Kafka | Self::Redis
        )
    }
}

/// Dispatch configuration
#[derive(Clone, Debug)]
pub struct DispatchConfig {
    /// Which backend to use for sync/streaming execution (/invoke)
    pub backend: ExecutionBackend,
    /// Which backend to use for async execution (/invoke/async)
    pub async_backend: ExecutionBackend,
    /// HTTP executor URL (for Http backend)
    pub executor_url: Option<String>,
    /// AWS Lambda function name/ARN (for Lambda backends)
    pub lambda_function_name: Option<String>,
    /// AWS region for Lambda
    pub lambda_region: Option<String>,
    /// Kubernetes namespace (for KubernetesJob backend)
    pub k8s_namespace: String,
    /// Kubernetes executor image
    pub k8s_executor_image: String,
    /// SQS queue URL (for Sqs backend)
    pub sqs_queue_url: Option<String>,
    /// SQS queue URL for EventBridge Pipe → ECS dispatch
    pub sqs_event_bridge_queue_url: Option<String>,
    /// Kafka bootstrap servers (comma-separated)
    pub kafka_brokers: Option<String>,
    /// Kafka topic name
    pub kafka_topic: Option<String>,
    /// Redis URL (for Redis queue backend)
    pub redis_url: Option<String>,
    /// Redis queue name
    pub redis_queue_name: String,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DispatchConfig {
    pub fn from_env() -> Self {
        Self {
            backend: ExecutionBackend::from_env(),
            async_backend: ExecutionBackend::async_from_env(),
            executor_url: std::env::var("EXECUTOR_URL").ok(),
            lambda_function_name: std::env::var("LAMBDA_EXECUTOR_FUNCTION").ok(),
            lambda_region: std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .ok(),
            k8s_namespace: std::env::var("K8S_NAMESPACE").unwrap_or_else(|_| "default".into()),
            k8s_executor_image: std::env::var("K8S_EXECUTOR_IMAGE")
                .unwrap_or_else(|_| "flow-like-executor:latest".into()),
            sqs_queue_url: std::env::var("SQS_EXECUTION_QUEUE_URL").ok(),
            sqs_event_bridge_queue_url: std::env::var("SQS_EVENT_BRIDGE_EXECUTION_QUEUE_URL").ok(),
            kafka_brokers: std::env::var("KAFKA_BROKERS").ok(),
            kafka_topic: std::env::var("KAFKA_EXECUTION_TOPIC").ok(),
            redis_url: std::env::var("REDIS_URL").ok(),
            redis_queue_name: std::env::var("REDIS_EXECUTION_QUEUE")
                .unwrap_or_else(|_| "exec:jobs".into()),
        }
    }
}

/// Request to dispatch an execution
/// The API is responsible for resolving events to board_id + board_version before dispatch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispatchRequest {
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
    /// Board version as tuple (major, minor, patch) - resolved by API
    pub board_version: Option<(u32, u32, u32)>,
    /// Node ID to start execution from
    pub node_id: String,
    /// Event data (serialized Event struct) if executing via event trigger
    pub event_json: Option<String>,
    pub payload: Option<serde_json::Value>,
    pub user_id: String,
    pub credentials_json: String,
    pub jwt: String,
    pub callback_url: String,
    /// User's auth token for the flow to use
    pub token: Option<String>,
    /// OAuth tokens keyed by provider name
    pub oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Whether to stream node state updates
    #[serde(default)]
    pub stream_state: bool,
    /// Runtime-configured variables to override board variables
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_variables:
        Option<std::collections::HashMap<String, flow_like::flow::variable::Variable>>,
    /// User execution context for permission checks during execution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_context: Option<flow_like::flow::execution::UserExecutionContext>,
    /// User profile data for execution context (bits, settings, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<serde_json::Value>,
    /// Pre-resolved WASM packages with presigned download URLs for executor
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_packages:
        Option<std::collections::HashMap<String, flow_like_types::dispatch::WasmPackageRef>>,
}

/// Response from dispatch
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispatchResponse {
    pub job_id: String,
    pub status: String,
    pub backend: String,
}

/// Dispatch errors
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("Configuration error: {0}")]
    Configuration(String),
    #[error("Network error: {0}")]
    Network(String),
    #[error("Lambda error: {0}")]
    Lambda(String),
    #[error("Kubernetes error: {0}")]
    Kubernetes(String),
    #[error("SQS error: {0}")]
    Sqs(String),
    #[error("Kafka error: {0}")]
    Kafka(String),
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Unified job dispatcher
#[derive(Clone)]
pub struct Dispatcher {
    config: Arc<DispatchConfig>,
    staging_bucket: Option<Arc<FlowLikeStore>>,
    #[cfg(feature = "lambda")]
    lambda_client: Option<aws_sdk_lambda::Client>,
    #[cfg(feature = "sqs")]
    sqs_client: Option<aws_sdk_sqs::Client>,
    #[cfg(feature = "redis")]
    redis_client: Option<redis::Client>,
}

impl Dispatcher {
    pub async fn new(config: DispatchConfig, staging_bucket: Option<Arc<FlowLikeStore>>) -> Self {
        // Determine if any AWS SDK clients are needed
        #[allow(unused_variables)]
        let needs_aws = {
            let mut needed = false;
            #[cfg(feature = "lambda")]
            {
                needed = needed || config.backend.is_lambda() || config.async_backend.is_lambda();
            }
            #[cfg(feature = "sqs")]
            {
                needed = needed
                    || config.backend == ExecutionBackend::Sqs
                    || config.async_backend == ExecutionBackend::Sqs
                    || config.backend == ExecutionBackend::SqsEventBridge
                    || config.async_backend == ExecutionBackend::SqsEventBridge;
            }
            needed
        };

        // Load AWS config once if any AWS client is needed
        #[cfg(any(feature = "lambda", feature = "sqs"))]
        let aws_config = if needs_aws {
            Some(aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await)
        } else {
            None
        };

        #[cfg(feature = "lambda")]
        let lambda_client = if config.backend.is_lambda() || config.async_backend.is_lambda() {
            aws_config.as_ref().map(|cfg| aws_sdk_lambda::Client::new(cfg))
        } else {
            None
        };

        #[cfg(feature = "sqs")]
        let sqs_client = if config.backend == ExecutionBackend::Sqs
            || config.async_backend == ExecutionBackend::Sqs
            || config.backend == ExecutionBackend::SqsEventBridge
            || config.async_backend == ExecutionBackend::SqsEventBridge
        {
            aws_config.as_ref().map(|cfg| aws_sdk_sqs::Client::new(cfg))
        } else {
            None
        };

        #[cfg(feature = "redis")]
        let redis_client = if config.backend == ExecutionBackend::Redis
            || config.async_backend == ExecutionBackend::Redis
        {
            config
                .redis_url
                .as_ref()
                .and_then(|url| redis::Client::open(url.as_str()).ok())
        } else {
            None
        };

        Self {
            config: Arc::new(config),
            staging_bucket,
            #[cfg(feature = "lambda")]
            lambda_client,
            #[cfg(feature = "sqs")]
            sqs_client,
            #[cfg(feature = "redis")]
            redis_client,
        }
    }

    pub fn from_config(config: DispatchConfig) -> Self {
        Self {
            config: Arc::new(config),
            staging_bucket: None,
            #[cfg(feature = "lambda")]
            lambda_client: None,
            #[cfg(feature = "sqs")]
            sqs_client: None,
            #[cfg(feature = "redis")]
            redis_client: None,
        }
    }

    /// Get the configured sync/streaming backend type
    pub fn backend(&self) -> ExecutionBackend {
        self.config.backend.clone()
    }

    /// Dispatch an execution request to the configured sync/streaming backend (EXECUTION_BACKEND)
    pub async fn dispatch(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        self.dispatch_to_backend(self.config.backend.clone(), request)
            .await
    }

    /// Dispatch an execution request to the configured async backend (ASYNC_EXECUTION_BACKEND)
    pub async fn dispatch_async(
        &self,
        request: DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        self.dispatch_to_backend(self.config.async_backend.clone(), request)
            .await
    }

    /// Dispatch an execution request to a specific backend (override default)
    pub async fn dispatch_with_backend(
        &self,
        backend: ExecutionBackend,
        request: DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        self.dispatch_to_backend(backend, request).await
    }

    async fn dispatch_to_backend(
        &self,
        backend: ExecutionBackend,
        request: DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        let job_id = create_id();

        match backend {
            ExecutionBackend::Http => self.dispatch_http(&job_id, &request).await,
            ExecutionBackend::LambdaInvoke => self.dispatch_lambda_invoke(&job_id, &request).await,
            ExecutionBackend::LambdaStream => Err(DispatchError::Configuration(
                "LambdaStream requires dispatch_streaming() method".into(),
            )),
            ExecutionBackend::KubernetesJob => self.dispatch_k8s_job(&job_id, &request).await,
            ExecutionBackend::Sqs => self.dispatch_sqs(&job_id, &request).await,
            ExecutionBackend::SqsEventBridge => {
                self.dispatch_sqs_event_bridge(&job_id, &request).await
            }
            ExecutionBackend::Kafka => self.dispatch_kafka(&job_id, &request).await,
            ExecutionBackend::Redis => self.dispatch_redis(&job_id, &request).await,
        }
    }

    /// Dispatch an execution request and return a streaming response
    /// Only supported for LambdaStream backend
    #[cfg(feature = "lambda")]
    pub async fn dispatch_streaming(
        &self,
        request: DispatchRequest,
    ) -> Result<(DispatchResponse, ByteStream), DispatchError> {
        let job_id = create_id();

        match self.config.backend {
            ExecutionBackend::LambdaStream => self.dispatch_lambda_stream(&job_id, &request).await,
            _ => Err(DispatchError::Configuration(format!(
                "Streaming dispatch not supported for {:?} backend. Use LambdaStream backend.",
                self.config.backend
            ))),
        }
    }

    #[cfg(not(feature = "lambda"))]
    pub async fn dispatch_streaming(
        &self,
        _request: DispatchRequest,
    ) -> Result<(DispatchResponse, ByteStream), DispatchError> {
        Err(DispatchError::Configuration(
            "Streaming dispatch requires the 'lambda' feature".into(),
        ))
    }

    /// Dispatch via HTTP POST to executor endpoint
    async fn dispatch_http(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        let url =
            self.config.executor_url.as_ref().ok_or_else(|| {
                DispatchError::Configuration("EXECUTOR_URL not configured".into())
            })?;

        let body = build_executor_payload(job_id, request);

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/execute", url))
            .json(&body)
            .send()
            .await
            .map_err(|e| DispatchError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(DispatchError::Network(format!("HTTP {}: {}", status, text)));
        }

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "dispatched".into(),
            backend: "http".into(),
        })
    }

    /// Dispatch via HTTP POST to executor SSE endpoint and return streaming response
    pub async fn dispatch_http_sse(
        &self,
        request: DispatchRequest,
    ) -> Result<(DispatchResponse, reqwest::Response), DispatchError> {
        let url =
            self.config.executor_url.as_ref().ok_or_else(|| {
                DispatchError::Configuration("EXECUTOR_URL not configured".into())
            })?;

        tracing::info!(url = %url, "Dispatching HTTP SSE");

        let job_id = create_id();
        let body = build_executor_payload(&job_id, &request);

        tracing::debug!(job_id = %job_id, "Dispatch payload built");

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/execute/sse", url))
            .json(&body)
            .send()
            .await
            .map_err(|e| DispatchError::Network(e.to_string()))?;

        let status = response.status();
        tracing::info!(job_id = %job_id, status = %status, "Executor responded");

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(DispatchError::Network(format!("HTTP {}: {}", status, text)));
        }

        let dispatch_response = DispatchResponse {
            job_id,
            status: "streaming".into(),
            backend: "http_sse".into(),
        };

        Ok((dispatch_response, response))
    }

    /// Dispatch via AWS Lambda SDK invocation (async, fire-and-forget)
    #[cfg(feature = "lambda")]
    async fn dispatch_lambda_invoke(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        let function_name = self.config.lambda_function_name.as_ref().ok_or_else(|| {
            DispatchError::Configuration("LAMBDA_EXECUTOR_FUNCTION not configured".into())
        })?;

        let client = self
            .lambda_client
            .as_ref()
            .ok_or_else(|| DispatchError::Configuration("Lambda client not initialized".into()))?;

        let body = build_executor_payload(job_id, request);
        // Wrap in API Gateway v2 event format for lambda_http compatibility
        let apigw_event = wrap_as_apigw_v2_event("/execute", body);
        let payload = serde_json::to_vec(&apigw_event)
            .map_err(|e| DispatchError::Serialization(e.to_string()))?;

        client
            .invoke()
            .function_name(function_name)
            .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload))
            .send()
            .await
            .map_err(|e| DispatchError::Lambda(e.to_string()))?;

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "invoked".into(),
            backend: "lambda_invoke".into(),
        })
    }

    /// Dispatch via AWS Lambda SDK streaming invocation
    #[cfg(feature = "lambda")]
    async fn dispatch_lambda_stream(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<(DispatchResponse, ByteStream), DispatchError> {
        use aws_sdk_lambda::types::InvokeWithResponseStreamResponseEvent;

        let function_name = self.config.lambda_function_name.as_ref().ok_or_else(|| {
            DispatchError::Configuration("LAMBDA_EXECUTOR_FUNCTION not configured".into())
        })?;

        let client = self
            .lambda_client
            .as_ref()
            .ok_or_else(|| DispatchError::Configuration("Lambda client not initialized".into()))?;

        let body = build_executor_payload(job_id, request);
        // Wrap in API Gateway v2 event format for lambda_http compatibility
        let apigw_event = wrap_as_apigw_v2_event("/execute/sse", body);
        let payload = serde_json::to_vec(&apigw_event)
            .map_err(|e| DispatchError::Serialization(e.to_string()))?;

        let response = client
            .invoke_with_response_stream()
            .function_name(function_name)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload))
            .send()
            .await
            .map_err(|e| DispatchError::Lambda(e.to_string()))?;

        let event_stream = response.event_stream;
        let stream = futures::stream::unfold(event_stream, |mut receiver| async move {
            match receiver.recv().await {
                Ok(Some(event)) => match event {
                    InvokeWithResponseStreamResponseEvent::PayloadChunk(chunk) => {
                        if let Some(payload) = chunk.payload {
                            Some((Ok(bytes::Bytes::from(payload.into_inner())), receiver))
                        } else {
                            Some((Ok(bytes::Bytes::new()), receiver))
                        }
                    }
                    InvokeWithResponseStreamResponseEvent::InvokeComplete(_) => None,
                    _ => Some((Ok(bytes::Bytes::new()), receiver)),
                },
                Ok(None) => None,
                Err(e) => Some((Err(DispatchError::Lambda(e.to_string())), receiver)),
            }
        });

        let response = DispatchResponse {
            job_id: job_id.to_string(),
            status: "streaming".into(),
            backend: "lambda_stream".into(),
        };

        Ok((response, Box::pin(stream)))
    }

    #[cfg(not(feature = "lambda"))]
    async fn dispatch_lambda_invoke(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "Lambda SDK invoke requires the 'lambda' feature. Use HTTP backend with Lambda Function URLs instead.".into(),
        ))
    }

    /// Dispatch via Kubernetes Job creation
    #[cfg(feature = "kubernetes")]
    async fn dispatch_k8s_job(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        use crate::kubernetes::{
            ExecutionContext, JobDispatcher, JobMode, KubernetesConfig, SubmitJobRequest,
        };

        let k8s_config = KubernetesConfig::from_env();
        let dispatcher = JobDispatcher::new(k8s_config);

        let k8s_request = SubmitJobRequest {
            run_id: request.run_id.clone(),
            app_id: request.app_id.clone(),
            board_id: request.board_id.clone(),
            event_id: None,
            version: request
                .board_version
                .map(|(major, minor, patch)| format!("{major}.{minor}.{patch}")),
            payload: request.payload.clone(),
            mode: JobMode::Isolated,
            user_id: request.user_id.clone(),
            execution_context: ExecutionContext {
                credentials_json: request.credentials_json.clone(),
                jwt: request.jwt.clone(),
                callback_url: request.callback_url.clone(),
            },
        };

        dispatcher
            .submit(k8s_request)
            .await
            .map_err(|e| DispatchError::Kubernetes(e.to_string()))?;

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "created".into(),
            backend: "kubernetes_job".into(),
        })
    }

    #[cfg(not(feature = "kubernetes"))]
    async fn dispatch_k8s_job(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "Kubernetes Job dispatch requires the 'kubernetes' feature".into(),
        ))
    }

    /// Dispatch via AWS SQS queue for batch processing
    #[cfg(feature = "sqs")]
    async fn dispatch_sqs(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        let queue_url = self.config.sqs_queue_url.as_ref().ok_or_else(|| {
            DispatchError::Configuration("SQS_EXECUTION_QUEUE_URL not configured".into())
        })?;

        let client = self
            .sqs_client
            .as_ref()
            .ok_or_else(|| DispatchError::Configuration("SQS client not initialized".into()))?;

        let body = build_executor_payload(job_id, request);
        let message_body = serde_json::to_string(&body)
            .map_err(|e| DispatchError::Serialization(e.to_string()))?;

        let mut req = client
            .send_message()
            .queue_url(queue_url)
            .message_body(&message_body)
            .message_group_id(&request.app_id);

        if queue_url.ends_with(".fifo") {
            req = req.message_deduplication_id(job_id);
        }

        req.send()
            .await
            .map_err(|e| DispatchError::Sqs(e.to_string()))?;

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "queued".into(),
            backend: "sqs".into(),
        })
    }

    #[cfg(not(feature = "sqs"))]
    async fn dispatch_sqs(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "SQS dispatch requires the 'sqs' feature".into(),
        ))
    }

    /// Dispatch via SQS → EventBridge Pipe → ECS RunTask.
    ///
    /// Stages the full payload to object storage as JSON, signs a GET URL,
    /// and sends only a compact `DispatchPayloadRef::Remote` reference via SQS.
    /// This avoids ECS container-override env var size limits (~8 KB).
    #[cfg(feature = "sqs")]
    async fn dispatch_sqs_event_bridge(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        let queue_url = self
            .config
            .sqs_event_bridge_queue_url
            .as_ref()
            .ok_or_else(|| {
                DispatchError::Configuration(
                    "SQS_EVENT_BRIDGE_EXECUTION_QUEUE_URL not configured".into(),
                )
            })?;

        let staging = self.staging_bucket.as_ref().ok_or_else(|| {
            DispatchError::Configuration(
                "Staging bucket not configured for SqsEventBridge backend".into(),
            )
        })?;

        let client = self
            .sqs_client
            .as_ref()
            .ok_or_else(|| DispatchError::Configuration("SQS client not initialized".into()))?;

        let body = build_executor_payload(job_id, request);
        let payload_bytes =
            serde_json::to_vec(&body).map_err(|e| DispatchError::Serialization(e.to_string()))?;

        let staging_path = StorePath::from(format!("tmp/sqs/{}.json", job_id));
        staging
            .put(&staging_path, payload_bytes)
            .await
            .map_err(|e| DispatchError::Sqs(format!("Failed to stage payload: {}", e)))?;

        let presigned_url = staging
            .sign("GET", &staging_path, std::time::Duration::from_secs(86400))
            .await
            .map_err(|e| DispatchError::Sqs(format!("Failed to sign staging URL: {}", e)))?;

        let reference = flow_like_types::dispatch::DispatchPayloadRef::Remote {
            remote_url: presigned_url.to_string(),
        };
        let message_body = serde_json::to_string(&reference)
            .map_err(|e| DispatchError::Serialization(e.to_string()))?;

        let mut req = client
            .send_message()
            .queue_url(queue_url)
            .message_body(&message_body)
            .message_group_id(&request.app_id);

        if queue_url.ends_with(".fifo") {
            req = req.message_deduplication_id(job_id);
        }

        req.send()
            .await
            .map_err(|e| DispatchError::Sqs(e.to_string()))?;

        tracing::info!(
            job_id = %job_id,
            staging_path = %staging_path,
            "Dispatched execution via SQS → EventBridge → ECS"
        );

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "queued".into(),
            backend: "sqs_event_bridge".into(),
        })
    }

    #[cfg(not(feature = "sqs"))]
    async fn dispatch_sqs_event_bridge(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "SQS EventBridge dispatch requires the 'sqs' feature".into(),
        ))
    }

    /// Dispatch via Apache Kafka for high-throughput batch processing
    async fn dispatch_kafka(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        let brokers =
            self.config.kafka_brokers.as_ref().ok_or_else(|| {
                DispatchError::Configuration("KAFKA_BROKERS not configured".into())
            })?;
        let topic = self.config.kafka_topic.as_ref().ok_or_else(|| {
            DispatchError::Configuration("KAFKA_EXECUTION_TOPIC not configured".into())
        })?;

        let body = build_executor_payload(job_id, request);
        let message_body = serde_json::to_string(&body)
            .map_err(|e| DispatchError::Serialization(e.to_string()))?;

        // Use HTTP to post to a Kafka REST proxy (e.g., Confluent REST Proxy)
        // This avoids adding heavy Kafka client dependencies
        let client = reqwest::Client::new();
        let proxy_url = format!("{}/topics/{}", brokers, topic);

        let kafka_message = serde_json::json!({
            "records": [{
                "key": request.app_id,
                "value": message_body
            }]
        });

        let response = client
            .post(&proxy_url)
            .header("Content-Type", "application/vnd.kafka.json.v2+json")
            .json(&kafka_message)
            .send()
            .await
            .map_err(|e| DispatchError::Kafka(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(DispatchError::Kafka(format!("HTTP {}: {}", status, text)));
        }

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "queued".into(),
            backend: "kafka".into(),
        })
    }

    /// Dispatch via Redis queue for Docker Compose / Kubernetes async dispatch
    #[cfg(feature = "redis")]
    async fn dispatch_redis(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        use redis::AsyncCommands;

        let client = self.redis_client.as_ref().ok_or_else(|| {
            DispatchError::Configuration("Redis client not initialized. Set REDIS_URL.".into())
        })?;

        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| DispatchError::Redis(e.to_string()))?;

        let body = build_executor_payload(job_id, request);
        let message_body = serde_json::to_string(&body)
            .map_err(|e| DispatchError::Serialization(e.to_string()))?;

        // Use LPUSH to add to the left side of the list (workers use BRPOP from right)
        let queue_name = &self.config.redis_queue_name;
        conn.lpush::<_, _, ()>(queue_name, &message_body)
            .await
            .map_err(|e| DispatchError::Redis(e.to_string()))?;

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "queued".into(),
            backend: "redis".into(),
        })
    }

    #[cfg(not(feature = "redis"))]
    async fn dispatch_redis(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "Redis dispatch requires the 'redis' feature".into(),
        ))
    }
}

/// Build the payload sent to the executor
fn build_executor_payload(job_id: &str, request: &DispatchRequest) -> serde_json::Value {
    let credentials: serde_json::Value = serde_json::from_str(&request.credentials_json)
        .unwrap_or_else(|_| serde_json::Value::String(request.credentials_json.clone()));

    let oauth_tokens = request.oauth_tokens.as_ref().map(|tokens| {
        tokens
            .iter()
            .filter_map(|(k, v)| {
                serde_json::from_value::<flow_like_types::OAuthTokenInput>(v.clone())
                    .ok()
                    .map(|t| (k.clone(), t))
            })
            .collect()
    });

    let payload = flow_like_types::dispatch::DispatchPayload {
        job_id: job_id.to_string(),
        run_id: request.run_id.clone(),
        app_id: request.app_id.clone(),
        board_id: request.board_id.clone(),
        board_version: request.board_version,
        node_id: request.node_id.clone(),
        event_json: request.event_json.clone(),
        payload: request.payload.clone(),
        user_id: request.user_id.clone(),
        credentials,
        executor_jwt: request.jwt.clone(),
        callback_url: request.callback_url.clone(),
        token: request.token.clone(),
        oauth_tokens,
        stream_state: request.stream_state,
        runtime_variables: request
            .runtime_variables
            .as_ref()
            .and_then(|v| serde_json::to_value(v).ok()),
        user_context: request
            .user_context
            .as_ref()
            .and_then(|v| serde_json::to_value(v).ok()),
        profile: request.profile.clone(),
        wasm_packages: request.wasm_packages.clone(),
    };

    serde_json::to_value(&payload).expect("Failed to serialize DispatchPayload")
}

/// Wrap executor payload in API Gateway v2 HTTP event format.
/// This is required when invoking Lambda functions that use `lambda_http`
/// (which expects API Gateway / Function URL event structure) via direct
/// Lambda SDK invocation rather than HTTP.
#[cfg(feature = "lambda")]
fn wrap_as_apigw_v2_event(path: &str, body: serde_json::Value) -> serde_json::Value {
    use aws_lambda_events::apigw::{
        ApiGatewayV2httpRequest, ApiGatewayV2httpRequestContext,
        ApiGatewayV2httpRequestContextHttpDescription,
    };
    use hyper::http::{HeaderMap, Method, header::CONTENT_TYPE};

    let body_string = serde_json::to_string(&body).unwrap_or_default();
    let body_base64 =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body_string);

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());

    let now = chrono::Utc::now();

    // Build the HTTP description for request context
    let mut http_desc = ApiGatewayV2httpRequestContextHttpDescription::default();
    http_desc.method = Method::POST;
    http_desc.path = Some(path.to_string());
    http_desc.protocol = Some("HTTP/1.1".to_string());
    http_desc.source_ip = Some("127.0.0.1".to_string());
    http_desc.user_agent = Some("flow-like-api/1.0".to_string());

    // Build the request context
    let mut request_context = ApiGatewayV2httpRequestContext::default();
    request_context.account_id = Some("anonymous".to_string());
    request_context.apiid = Some("lambda-invoke".to_string());
    request_context.domain_name = Some("lambda.internal".to_string());
    request_context.domain_prefix = Some("lambda-invoke".to_string());
    request_context.http = http_desc;
    request_context.request_id = Some(flow_like_types::create_id());
    request_context.route_key = Some("$default".to_string());
    request_context.stage = Some("$default".to_string());
    request_context.time = Some(now.format("%d/%b/%Y:%H:%M:%S %z").to_string());
    request_context.time_epoch = now.timestamp_millis();

    // Build the full request
    let mut request = ApiGatewayV2httpRequest::default();
    request.version = Some("2.0".to_string());
    request.route_key = Some("$default".to_string());
    request.raw_path = Some(path.to_string());
    request.raw_query_string = Some(String::new());
    request.headers = headers;
    request.request_context = request_context;
    request.body = Some(body_base64);
    request.is_base64_encoded = true;

    serde_json::to_value(&request).expect("Failed to serialize API Gateway event")
}

/// Fetch a profile for a user from the database and convert it
/// to a JSON value matching the core `Profile` struct format for the executor.
///
/// Resolution order:
/// 1. If `profile_id` is provided, fetch that specific profile (must belong to user)
/// 2. Otherwise find the first profile whose `apps` list contains the given `app_id`
/// 3. Fallback to the first profile for the user
pub async fn fetch_profile_for_dispatch(
    db: &sea_orm::DatabaseConnection,
    user_id: &str,
    profile_id: Option<&str>,
    app_id: &str,
) -> Option<serde_json::Value> {
    use crate::entity::profile;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let model = if let Some(pid) = profile_id {
        profile::Entity::find_by_id(pid)
            .filter(profile::Column::UserId.eq(user_id))
            .one(db)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    let model = if model.is_some() {
        model
    } else {
        let profiles = profile::Entity::find()
            .filter(profile::Column::UserId.eq(user_id))
            .all(db)
            .await
            .ok()
            .unwrap_or_default();

        profiles
            .iter()
            .find(|p| {
                p.apps
                    .as_ref()
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .any(|a| a.get("app_id").and_then(|id| id.as_str()) == Some(app_id))
                    })
                    .unwrap_or(false)
            })
            .or_else(|| profiles.first())
            .cloned()
    };

    let model = model?;

    Some(serde_json::json!({
        "id": model.id,
        "name": model.name,
        "description": model.description,
        "icon": model.icon,
        "thumbnail": model.thumbnail,
        "interests": model.interests.unwrap_or_default(),
        "tags": model.tags.unwrap_or_default(),
        "hub": model.hub,
        "secure": true,
        "hubs": model.hubs.unwrap_or_default(),
        "apps": model.apps,
        "shortcuts": model.shortcuts,
        "theme": model.theme,
        "bits": model.bit_ids.unwrap_or_default(),
        "settings": model.settings.unwrap_or_else(|| serde_json::json!({"connection_mode": "simplebezier"})),
        "updated": model.updated_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        "created": model.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
    }))
}
