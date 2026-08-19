//! Unified job dispatcher supporting multiple execution backends.
//!
//! ## Configuration
//!
//! Two environment variables control dispatch behavior:
//!
//! - **`EXECUTION_BACKEND`**: Used for `/invoke` (streaming/sync) endpoints
//! - **`ASYNC_EXECUTION_BACKEND`**: Used for `/invoke/async` endpoints
//!
//! A third variable, **`LAMBDA_TENANT_ISOLATION`**, is read only by the Lambda
//! backends and is documented under "Isolation & Security Model" below.
//!
//! Both support the same backend options, allowing different backends for
//! realtime streaming vs background batch processing. Accepted values are
//! `lambda_invoke`, `lambda_stream`, `kubernetes_job`, `sqs`, `azure_queue`,
//! `pubsub`, `sqs_event_bridge`, `kafka` and `redis`; anything else — including
//! an unset variable — selects `http`. `from_env_var` lists the aliases each
//! value accepts.
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
//! - **AzureQueue**: Azure Queue Storage queue with managed-identity workers
//! - **PubSub**: Google Cloud Pub/Sub topic with workload-identity subscribers
//! - **SqsEventBridge**: SQS → EventBridge Pipe → ECS RunTask (payload staged to storage)
//! - **Kafka**: Apache Kafka queue for high-throughput batch processing
//! - **Redis**: Redis queue for Docker Compose / Kubernetes async dispatch
//!
//! ## Isolation & Security Model
//!
//! ### Lambda (HTTP / LambdaInvoke / LambdaStream)
//!
//! Every execution runs inside a Firecracker microVM, so a run is isolated from
//! the host and from the runs executing beside it. It is **not** isolated from
//! the runs that came before it: by default a warm execution environment is
//! reused across invocations of the same function no matter which subject
//! triggered them, and it carries process memory and a `/tmp` scratch directory
//! across that reuse.
//!
//! `LAMBDA_TENANT_ISOLATION=sub` closes that gap on the `LambdaInvoke` and
//! `LambdaStream` backends. Each invocation carries a tenant id derived from the
//! subject, and AWS binds an execution environment to one tenant for its
//! lifetime rather than reusing it for another. Three constraints come with it:
//! the executor function must have been created with
//! `TenancyConfig.TenantIsolationMode=PER_TENANT` (a create-only property, so it
//! cannot be added to a running function), the `Http` backend cannot participate
//! because Lambda Function URLs do not support tenant isolation, and warm
//! capacity stops being shared — AWS caps tenant-bound environments at 2,500 per
//! 1,000 configured concurrency and bills each environment creation.
//!
//! **Best for**: Multi-tenant workloads. Set the flag when runs from different
//! subjects must not share an execution environment.
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
//! | Multi-tenant SaaS | Lambda with `LAMBDA_TENANT_ISOLATION=sub` |
//! | Low latency | HTTP → Warm Pool (K8s/Lambda) |
//! | Untrusted code | KubernetesJob or Lambda |
//! | Batch processing | SQS, Pub/Sub, Kafka, or Redis (decoupled, retry built-in) |
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
    /// Azure Queue Storage queue for batch processing (managed identity only)
    AzureQueue,
    /// Google Cloud Pub/Sub topic for batch processing (metadata-server
    /// workload identity only; no service account key is ever accepted)
    PubSub,
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
            "azure_queue" | "azure_storage_queue" | "queue_storage" => Self::AzureQueue,
            // `pub_sub` is the snake_case form this enum serializes to, so a
            // value round-tripped through a stored config resolves back to the
            // same backend it was written from.
            "pubsub" | "pub_sub" | "gcp_pubsub" => Self::PubSub,
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
            Self::Sqs
                | Self::AzureQueue
                | Self::PubSub
                | Self::SqsEventBridge
                | Self::Kafka
                | Self::Redis
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
    /// Storage account hosting the work queues (for AzureQueue backend)
    pub queue_account_name: Option<String>,
    /// Azure Queue Storage execution queue name
    pub queue_name: Option<String>,
    /// Google Cloud project that owns the Pub/Sub topics (for PubSub backend)
    pub pubsub_project: Option<String>,
    /// Pub/Sub execution topic, either a bare topic ID or the fully qualified
    /// `projects/<project>/topics/<topic>` resource name the deployment emits
    pub pubsub_topic: Option<String>,
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
            queue_account_name: std::env::var("AZURE_QUEUE_STORAGE_ACCOUNT_NAME").ok(),
            queue_name: std::env::var("AZURE_QUEUE_EXECUTION").ok(),
            // `GCP_PUBSUB_PROJECT` takes precedence so a deployment can keep its
            // messaging plane in a project separate from the one the workload
            // itself runs in; the standard deployment sets both to the same
            // value. Whichever wins is checked against the topic resource name
            // before publishing, so a half-migrated pair cannot silently send
            // jobs into the wrong project's topic.
            pubsub_project: std::env::var("GCP_PUBSUB_PROJECT")
                .or_else(|_| std::env::var("GCP_PROJECT_ID"))
                .ok(),
            pubsub_topic: std::env::var("PUBSUB_EXECUTION_TOPIC").ok(),
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
    /// Execution mode reported inside the flow runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<flow_like::flow::execution::ExecutionMode>,
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
    #[error("Azure Queue Storage error: {0}")]
    AzureQueue(String),
    #[error("Pub/Sub error: {0}")]
    PubSub(String),
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
            aws_config
                .as_ref()
                .map(|cfg| aws_sdk_lambda::Client::new(cfg))
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
            ExecutionBackend::AzureQueue => self.dispatch_azure_queue(&job_id, &request).await,
            ExecutionBackend::PubSub => self.dispatch_pubsub(&job_id, &request).await,
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
        let response = attach_executor_iam_auth(client.post(format!("{}/execute", url)), url)
            .await?
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
        let response = attach_executor_iam_auth(client.post(format!("{}/execute/sse", url)), url)
            .await?
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

    /// The tenant id this invocation must carry, or `None` when the deployment
    /// has not opted in.
    ///
    /// Resolved from `LAMBDA_TENANT_ISOLATION` rather than from `DispatchConfig`:
    /// `DispatchConfig::from_env` is infallible by construction, so a flag
    /// parsed there could only swallow a bad value or panic, and swallowing is
    /// the one outcome this gate must not have.
    #[cfg(feature = "lambda")]
    fn lambda_tenant_id_for(
        &self,
        request: &DispatchRequest,
    ) -> Result<Option<String>, DispatchError> {
        let enabled = *LAMBDA_TENANT_ISOLATION
            .as_ref()
            .map_err(|error| DispatchError::Configuration(error.clone()))?;
        if !enabled {
            return Ok(None);
        }

        let tenant_id = lambda_tenant_id(&request.user_id);
        // The subject never reaches AWS, so this line is the only place the
        // mapping exists — without it a tenant id in CloudWatch cannot be
        // traced back to the run that produced it.
        tracing::debug!(
            run_id = %request.run_id,
            user_id = %request.user_id,
            tenant_id = %tenant_id,
            "Lambda tenant isolation active"
        );
        Ok(Some(tenant_id))
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

        let tenant_id = self.lambda_tenant_id_for(request)?;

        let mut invoke = client
            .invoke()
            .function_name(function_name)
            .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload));
        if let Some(tenant_id) = tenant_id.as_deref() {
            invoke = invoke.tenant_id(tenant_id);
        }

        invoke
            .send()
            .await
            .map_err(|e| lambda_dispatch_error(e, tenant_id.is_some()))?;

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

        let tenant_id = self.lambda_tenant_id_for(request)?;

        let mut invoke = client
            .invoke_with_response_stream()
            .function_name(function_name)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload));
        if let Some(tenant_id) = tenant_id.as_deref() {
            invoke = invoke.tenant_id(tenant_id);
        }

        let response = invoke
            .send()
            .await
            .map_err(|e| lambda_dispatch_error(e, tenant_id.is_some()))?;

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

    /// Dispatch via Azure Queue Storage using only the API's user-assigned
    /// managed identity. Queue Storage mints its own message ID and offers no
    /// duplicate detection, so the job ID travels inside the message envelope.
    ///
    /// Payloads at or above `CLAIM_CHECK_THRESHOLD_BYTES` are staged to Blob
    /// Storage and sent as a compact `DispatchPayloadRef::Remote` reference;
    /// smaller payloads keep the direct single-hop inline path.
    #[cfg(feature = "storage-queue")]
    async fn dispatch_azure_queue(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        use crate::storage_queue::WORKLOAD_EXECUTION;

        let account_name = self.config.queue_account_name.as_deref().ok_or_else(|| {
            DispatchError::Configuration("AZURE_QUEUE_STORAGE_ACCOUNT_NAME not configured".into())
        })?;
        let queue_name = self.config.queue_name.as_deref().ok_or_else(|| {
            DispatchError::Configuration("AZURE_QUEUE_EXECUTION not configured".into())
        })?;

        let body = build_executor_payload(job_id, request);
        let payload_bytes =
            serde_json::to_vec(&body).map_err(|e| DispatchError::Serialization(e.to_string()))?;

        if crate::storage_queue::requires_claim_check(
            payload_bytes.len(),
            job_id,
            &request.app_id,
            WORKLOAD_EXECUTION,
        ) {
            let staging = self.staging_bucket.as_ref().ok_or_else(|| {
                DispatchError::Configuration(
                    "Staging bucket not configured for AzureQueue backend".into(),
                )
            })?;

            let staging_path = StorePath::from(format!("tmp/execution/{}.json", job_id));
            staging
                .put(&staging_path, payload_bytes)
                .await
                .map_err(|e| {
                    DispatchError::AzureQueue(format!("Failed to stage payload: {}", e))
                })?;

            // Deliberately outlives the queue's message TTL — see
            // `CLAIM_CHECK_URL_TTL` for the margin it carries.
            let presigned_url = staging
                .sign(
                    "GET",
                    &staging_path,
                    crate::storage_queue::CLAIM_CHECK_URL_TTL,
                )
                .await
                .map_err(|e| {
                    DispatchError::AzureQueue(format!("Failed to sign staging URL: {}", e))
                })?;

            let reference = flow_like_types::dispatch::DispatchPayloadRef::Remote {
                remote_url: presigned_url.to_string(),
            };
            crate::storage_queue::send_json(
                account_name,
                queue_name,
                job_id,
                &request.app_id,
                WORKLOAD_EXECUTION,
                &reference,
            )
            .await
            .map_err(DispatchError::AzureQueue)?;

            tracing::info!(
                job_id = %job_id,
                staging_path = %staging_path,
                "Dispatched execution via Azure Queue Storage (staged to Blob Storage)"
            );
        } else {
            crate::storage_queue::send_json(
                account_name,
                queue_name,
                job_id,
                &request.app_id,
                WORKLOAD_EXECUTION,
                &body,
            )
            .await
            .map_err(DispatchError::AzureQueue)?;
        }

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "queued".into(),
            backend: "azure_queue".into(),
        })
    }

    #[cfg(not(feature = "storage-queue"))]
    async fn dispatch_azure_queue(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "Azure Queue Storage dispatch requires the 'storage-queue' feature".into(),
        ))
    }

    /// Dispatch via Google Cloud Pub/Sub using only the metadata-server identity
    /// bound to the running revision. Pub/Sub mints its own `messageId` and
    /// offers no publisher-side deduplication, so the job ID travels inside the
    /// message envelope, exactly as it does on Queue Storage.
    ///
    /// Payloads at or above `CLAIM_CHECK_THRESHOLD_BYTES` are staged to Cloud
    /// Storage and sent as a compact `DispatchPayloadRef::Remote` reference;
    /// smaller payloads keep the direct single-hop inline path.
    #[cfg(feature = "pubsub")]
    async fn dispatch_pubsub(
        &self,
        job_id: &str,
        request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        use self::pubsub::WORKLOAD_EXECUTION;

        let project_id = self.config.pubsub_project.as_deref().ok_or_else(|| {
            DispatchError::Configuration(
                "GCP_PROJECT_ID (or GCP_PUBSUB_PROJECT) not configured".into(),
            )
        })?;
        let topic = self.config.pubsub_topic.as_deref().ok_or_else(|| {
            DispatchError::Configuration("PUBSUB_EXECUTION_TOPIC not configured".into())
        })?;

        let body = build_executor_payload(job_id, request);
        let payload_bytes =
            serde_json::to_vec(&body).map_err(|e| DispatchError::Serialization(e.to_string()))?;

        if pubsub::requires_claim_check(
            payload_bytes.len(),
            job_id,
            &request.app_id,
            WORKLOAD_EXECUTION,
        ) {
            let staging = self.staging_bucket.as_ref().ok_or_else(|| {
                DispatchError::Configuration(
                    "Staging bucket not configured for PubSub backend".into(),
                )
            })?;

            let staging_path = StorePath::from(format!("tmp/execution/{}.json", job_id));
            staging
                .put(&staging_path, payload_bytes)
                .await
                .map_err(|e| DispatchError::PubSub(format!("Failed to stage payload: {}", e)))?;

            // Sized to cover the subscription's retention window — see
            // `CLAIM_CHECK_URL_TTL` for the seven-day ceiling GCS puts on that
            // and the retention invariant the deployment has to hold up.
            let presigned_url = staging
                .sign("GET", &staging_path, pubsub::CLAIM_CHECK_URL_TTL)
                .await
                .map_err(|e| DispatchError::PubSub(format!("Failed to sign staging URL: {}", e)))?;

            let reference = flow_like_types::dispatch::DispatchPayloadRef::Remote {
                remote_url: presigned_url.to_string(),
            };
            pubsub::send_json(
                project_id,
                topic,
                job_id,
                &request.app_id,
                WORKLOAD_EXECUTION,
                &reference,
            )
            .await
            .map_err(DispatchError::PubSub)?;

            tracing::info!(
                job_id = %job_id,
                staging_path = %staging_path,
                "Dispatched execution via Pub/Sub (staged to Cloud Storage)"
            );
        } else {
            pubsub::send_json(
                project_id,
                topic,
                job_id,
                &request.app_id,
                WORKLOAD_EXECUTION,
                &body,
            )
            .await
            .map_err(DispatchError::PubSub)?;
        }

        Ok(DispatchResponse {
            job_id: job_id.to_string(),
            status: "queued".into(),
            backend: "pubsub".into(),
        })
    }

    #[cfg(not(feature = "pubsub"))]
    async fn dispatch_pubsub(
        &self,
        _job_id: &str,
        _request: &DispatchRequest,
    ) -> Result<DispatchResponse, DispatchError> {
        Err(DispatchError::Configuration(
            "Pub/Sub dispatch requires the 'pubsub' feature".into(),
        ))
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
        execution_mode: request.execution_mode.map(|mode| mode.as_str().to_string()),
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

/// Environment switch for the credential a synchronous executor request must
/// carry in front of the application payload. The only recognized value today
/// is `gcp_id_token`; anything else — including unset — means the executor is
/// reached without a transport-layer credential of its own.
const EXECUTOR_AUTH_VAR: &str = "EXECUTOR_AUTH";
const EXECUTOR_AUTH_GCP_ID_TOKEN: &str = "gcp_id_token";

/// Whether `EXECUTOR_AUTH` demands a Google ID token on every synchronous
/// executor request. Whitespace and case are tolerated so a value that passed
/// through Terraform templating or a shell export cannot silently disable the
/// IAM credential and turn every dispatch into a front-door 403.
fn executor_auth_requires_gcp_id_token(executor_auth: Option<&str>) -> bool {
    executor_auth
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case(EXECUTOR_AUTH_GCP_ID_TOKEN))
}

/// Attach the credential Cloud Run's IAM front door demands before a
/// synchronous executor request is sent.
///
/// On GCP the executor service admits only this API's service account as
/// `roles/run.invoker`, and Cloud Run authenticates the caller from a
/// Google-signed ID token whose audience is the service URL — a control that
/// is independent of ingress restrictions and of the backend JWT inside the
/// payload. That JWT stays untouched: it authenticates the job to the executor
/// application, this header authenticates the request to Cloud Run's IAM
/// layer.
///
/// A metadata-server failure is an error, never a silent skip: a dispatch
/// without the header would only travel on to a guaranteed 403 at the
/// executor's front door, reported as an opaque network failure instead of the
/// missing credential.
#[cfg(feature = "pubsub")]
async fn attach_executor_iam_auth(
    builder: reqwest::RequestBuilder,
    executor_url: &str,
) -> Result<reqwest::RequestBuilder, DispatchError> {
    if !executor_auth_requires_gcp_id_token(std::env::var(EXECUTOR_AUTH_VAR).ok().as_deref()) {
        return Ok(builder);
    }

    let audience = gcp_id_token::audience_from_executor_url(executor_url)
        .map_err(DispatchError::Configuration)?;
    let token = gcp_id_token::fetch_id_token(&audience)
        .await
        .map_err(DispatchError::Network)?;
    Ok(builder.bearer_auth(token))
}

/// Without the GCP dispatch surface compiled in there is no metadata-server
/// client to mint the token, so a deployment demanding one is refused outright
/// rather than dispatched into a certain 403.
#[cfg(not(feature = "pubsub"))]
async fn attach_executor_iam_auth(
    builder: reqwest::RequestBuilder,
    _executor_url: &str,
) -> Result<reqwest::RequestBuilder, DispatchError> {
    if executor_auth_requires_gcp_id_token(std::env::var(EXECUTOR_AUTH_VAR).ok().as_deref()) {
        return Err(DispatchError::Configuration(
            "EXECUTOR_AUTH=gcp_id_token requires the 'pubsub' feature".into(),
        ));
    }
    Ok(builder)
}

/// Environment switch that binds each executor invocation to a tenant-specific
/// Lambda execution environment. Unset means no tenant id is sent at all, which
/// is the only shape a function created without `TenancyConfig` accepts.
#[cfg(any(feature = "lambda", test))]
const LAMBDA_TENANT_ISOLATION_VAR: &str = "LAMBDA_TENANT_ISOLATION";

/// Domain separator mixed into every tenant digest.
///
/// `storage_path_segment` already derives its disambiguating suffix from a bare
/// `blake3::hash(sub)`, so an undomained digest here would open with the same
/// twelve hex characters as that subject's storage path and let either value
/// confirm the other. The `v1` names the derivation: changing it re-tenants
/// every caller and buys a full round of cold starts, so it has to be a
/// deliberate edit rather than an incidental one.
#[cfg(any(feature = "lambda", test))]
const LAMBDA_TENANT_ID_DOMAIN: &str = "flow-like:lambda-tenant:v1:";

/// Hex characters kept from the tenant digest. 32 is 128 bits, far past the
/// point where two subjects could collide into one execution environment.
#[cfg(any(feature = "lambda", test))]
const LAMBDA_TENANT_ID_HEX_CHARS: usize = 32;

/// Whether `LAMBDA_TENANT_ISOLATION` asks for per-subject execution
/// environments.
///
/// An unrecognized value is refused rather than read as `false`. The backend
/// parser above can afford to fall back to `http` on a typo because the wrong
/// transport announces itself immediately; this flag cannot, because failing
/// open costs exactly the isolation the operator believes they configured and
/// nothing in a successful invocation reports its absence.
#[cfg(any(feature = "lambda", test))]
fn lambda_tenant_isolation_enabled(raw: Option<&str>) -> Result<bool, String> {
    let Some(value) = raw.map(str::trim) else {
        return Ok(false);
    };

    match value.to_ascii_lowercase().as_str() {
        "" | "off" | "false" | "0" | "none" | "disabled" => Ok(false),
        "sub" | "user" | "user_id" | "true" | "1" | "on" | "enabled" => Ok(true),
        other => Err(format!(
            "{LAMBDA_TENANT_ISOLATION_VAR}={other:?} is not a recognized value; use `sub` to \
             give every subject its own execution environment, or `off` to disable"
        )),
    }
}

/// The parsed flag, resolved once.
///
/// The environment cannot change under a running process, and `std::env::var`
/// takes a process-wide lock and allocates on every read, so the parse is
/// memoized rather than repeated per dispatch. A rejected value is cached as
/// the error string and re-reported on every attempt, which keeps a
/// misconfigured deployment failing consistently instead of only on the first
/// invocation.
#[cfg(feature = "lambda")]
static LAMBDA_TENANT_ISOLATION: std::sync::LazyLock<Result<bool, String>> =
    std::sync::LazyLock::new(|| {
        lambda_tenant_isolation_enabled(std::env::var(LAMBDA_TENANT_ISOLATION_VAR).ok().as_deref())
    });

/// The `X-Amz-Tenant-Id` value Lambda routes an execution by.
///
/// The subject is hashed rather than passed through. AWS accepts only
/// `[a-zA-Z0-9._:/=+\-@ ]` in a tenant id, which excludes the `|` that
/// federated subjects such as `auth0|123` carry and that `validate_path_component`
/// deliberately admits; a raw subject would also land in the `tenantId` field of
/// CloudWatch platform events, readable by anyone holding log access; and a
/// digest is case-stable where AWS leaves tenant-id matching undocumented.
///
/// The derivation is total. Every subject yields a valid id, including the
/// `sink:` and `inbound:` placeholders that five of the dispatch routes
/// substitute when no user is attached — those isolate per sink and per event
/// definition rather than per user, which is the intended reading of a run that
/// has no user, not a defect.
#[cfg(any(feature = "lambda", test))]
fn lambda_tenant_id(subject: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LAMBDA_TENANT_ID_DOMAIN.as_bytes());
    hasher.update(subject.as_bytes());
    format!(
        "u{}",
        &hasher.finalize().to_hex()[..LAMBDA_TENANT_ID_HEX_CHARS]
    )
}

/// Render an AWS SDK failure with its full source chain.
///
/// `SdkError`'s own `Display` prints the bare string `"service error"` — the
/// modelled exception, the status and the message all hang off `source()`.
/// A tenancy mismatch arrives as `InvalidParameterValueException`, so without
/// the chain an operator whose executor function predates `TenancyConfig` reads
/// `Lambda error: service error` and has nothing to act on. AWS reports the
/// mismatch identically in both directions and does not publish the message
/// text, so the tenancy hint is appended from local state rather than parsed
/// out of the response.
#[cfg(feature = "lambda")]
fn lambda_dispatch_error<E: std::error::Error>(error: E, tenant_isolated: bool) -> DispatchError {
    let rendered = aws_sdk_lambda::error::DisplayErrorContext(error).to_string();
    if !tenant_isolated {
        return DispatchError::Lambda(rendered);
    }

    DispatchError::Lambda(format!(
        "{rendered} — {LAMBDA_TENANT_ISOLATION_VAR} is enabled, so the executor function must \
         have been created with TenancyConfig.TenantIsolationMode=PER_TENANT; that property is \
         create-only and cannot be added to an existing function"
    ))
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

/// Passwordless Google Cloud Pub/Sub publishing for queue-backed workloads.
///
/// This module intentionally supports only the workload identity the metadata
/// server hands out. Service account key JSON, `gcloud` cached tokens, emulator
/// hosts and API endpoint overrides are all refused on the production API path:
/// the only thing that authenticates a publish is an OAuth2 token minted for
/// the service account bound to the running revision, and IAM on the topic —
/// `roles/pubsub.publisher`, granted on the execution topic alone — is what
/// bounds what that token can do, not the OAuth scope it carries.
///
/// Pub/Sub's only Rust client is a generated SDK this crate does not otherwise
/// link, so the single `projects.topics.publish` call is issued directly
/// against the documented REST contract, in the same style as the `Put Message`
/// call in `crate::storage_queue`.
#[cfg(feature = "pubsub")]
pub(crate) mod pubsub {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use serde::{Deserialize, Serialize};
    use std::sync::OnceLock;
    use std::time::Duration;

    /// Publish endpoint. Deliberately a constant with no environment override:
    /// a deployment behind VPC Service Controls reaches Pub/Sub by resolving
    /// this hostname to the `restricted.googleapis.com` VIP through a private
    /// DNS zone, so the redirection already happens below this code. An
    /// override could therefore only ever serve to point a live service-account
    /// token at a host Google does not operate.
    const PUBSUB_ENDPOINT: &str = "https://pubsub.googleapis.com";

    /// Hard ceiling on the pre-base64 message body. Pub/Sub caps a publish
    /// request at 10 MB and the REST transport carries `data` base64-encoded
    /// inside a JSON document, which expands 3:4, so 7_500_000 bytes is the
    /// largest body that can still be published over this API. Exceeding it is
    /// a caller bug: anything at or above [`CLAIM_CHECK_THRESHOLD_BYTES`] must
    /// already have been staged to Cloud Storage and replaced by a claim-check
    /// reference.
    const MAX_MESSAGE_DATA_BYTES: usize = 10_000_000 * 3 / 4;
    /// Envelopes at or above this size are written to Cloud Storage and
    /// replaced by a claim-check reference instead of being inlined.
    ///
    /// Unlike the Azure threshold this is *not* derived from the wire limit —
    /// 7.5 MB is two orders of magnitude above any real payload. It is derived
    /// from retention fan-out: a published message is held by the subscription
    /// for its whole retention window, copied to the dead-letter topic once the
    /// delivery attempts are exhausted, and copied a third time by the sink
    /// that gives the dead-letter topic an evidence window longer than Pub/Sub
    /// can hold on its own. An inlined body is therefore stored several times
    /// over, for days, in places with no lifecycle rule of their own. 256 KiB
    /// sits roughly an order of magnitude above the measured 8-40 KiB execution
    /// envelope, so every ordinary dispatch keeps the single-hop inline path
    /// and only genuine outliers pay for the staging round trip.
    pub(crate) const CLAIM_CHECK_THRESHOLD_BYTES: usize = 256 * 1024;
    /// Lifetime of the signed URL a claim-check message points at.
    ///
    /// Seven days is the **maximum** a GCS V4 signature accepts, not a chosen
    /// margin, and it is the one place this path is tighter than the queue it
    /// mirrors: on Queue Storage the signature outlives the message TTL by six
    /// hours, here it can at best equal the broker's own ceiling. The
    /// deployment must therefore keep the execution subscription's
    /// `message_retention_duration` below seven days. If it does not, a message
    /// first delivered at the very end of its retention window points at an
    /// expired URL — which fails closed, because the fetch is refused and the
    /// worker dead-letters the job rather than executing a truncated payload.
    pub(crate) const CLAIM_CHECK_URL_TTL: Duration = Duration::from_secs(7 * 86_400);

    /// Envelope version understood by the queue worker. Pub/Sub mints its own
    /// `messageId` and its attributes are an unauthenticated side channel, so
    /// the job identity the worker checks the resolved payload against has to
    /// travel inside the body.
    const ENVELOPE_VERSION: u8 = 1;

    pub(crate) const WORKLOAD_EXECUTION: &str = "execution";
    pub(crate) const WORKLOAD_COMPILATION: &str = "compilation";
    /// Fed by Cloud Storage notifications in the deployment; the API never
    /// publishes into these topics today, but the worker accepts the names and
    /// a future re-enqueue path must not be rejected here.
    pub(crate) const WORKLOAD_FILE_TRACKING: &str = "file-tracking";
    pub(crate) const WORKLOAD_MEDIA_TRANSFORMATION: &str = "media-transformation";
    const SUPPORTED_WORKLOADS: [&str; 4] = [
        WORKLOAD_EXECUTION,
        WORKLOAD_COMPILATION,
        WORKLOAD_FILE_TRACKING,
        WORKLOAD_MEDIA_TRANSFORMATION,
    ];

    /// Metadata-server endpoint that mints an OAuth2 token for the service
    /// account bound to the running Cloud Run revision, GCE instance or GKE pod.
    const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
    const DEFAULT_METADATA_HOST: &str = "metadata.google.internal";
    const DEFAULT_METADATA_IP: &str = "169.254.169.254";
    const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
    const METADATA_FLAVOR_VALUE: &str = "Google";
    /// Narrow on purpose. Cloud Run hands out a `cloud-platform` token by
    /// default, which would open Secret Manager, GCS and Cloud SQL to anything
    /// that got hold of a copy; this token's only consumer is the Pub/Sub
    /// publish, so the publish scope alone is all it ever needs.
    const PUBSUB_SCOPE: &str = "https://www.googleapis.com/auth/pubsub";
    /// The narrowing request, and the fallback that keeps dispatch alive if the
    /// platform refuses it. `?scopes=` alone is silently ignored — the metadata
    /// server honours it only alongside `enforce_scopes=true` — and requesting
    /// non-default scopes is in turn reported not to work under GKE Workload
    /// Identity, so a rejected narrow request retries unnarrowed against the same
    /// authority instead of costing the dispatch.
    const NARROWED_TOKEN_QUERY: &[(&str, &str)] =
        &[("scopes", PUBSUB_SCOPE), ("enforce_scopes", "true")];
    const UNNARROWED_TOKEN_QUERY: &[(&str, &str)] = &[];
    /// A cached token is only reused while this much life remains. A publish
    /// that starts just before expiry and is rejected mid-flight leaves the
    /// worst of both states behind: the payload has already been staged to
    /// Cloud Storage but no message references it, so the object is an orphan
    /// and the run never starts. Five minutes is far more than a publish needs
    /// and costs nothing.
    const METADATA_TOKEN_MIN_LIFETIME_SECONDS: i64 = 5 * 60;
    /// Ceiling on cache residency. Metadata tokens live ~1h and the metadata
    /// server refreshes its own copy near the end of that window; re-asking
    /// every ten minutes keeps this process close to a full-lifetime token
    /// without turning every dispatch into a metadata round trip.
    const METADATA_TOKEN_CACHE_TTL_SECONDS: u64 = 10 * 60;
    pub(super) const METADATA_CONNECT_TIMEOUT_SECONDS: u64 = 3;
    pub(super) const METADATA_REQUEST_TIMEOUT_SECONDS: u64 = 10;

    const MAX_ERROR_BODY_BYTES: usize = 2_048;
    /// Settings that would move this publish off the metadata identity or off
    /// Google's own endpoint.
    ///
    /// `PUBSUB_EMULATOR_HOST` cannot redirect this module — the endpoint above
    /// is a constant — but its presence means something else in the same
    /// environment is pointed at an emulator. The API would then publish to the
    /// real topic while a worker read from a local one, and every dispatched
    /// job would silently disappear. Refusing the dispatch is the only way that
    /// misconfiguration ever surfaces. The rest are keys and pre-minted tokens:
    /// accepting them would let an operator quietly downgrade a keyless
    /// deployment to a long-lived credential without the deployment noticing.
    const FORBIDDEN_CREDENTIAL_SETTINGS: &[&str] = &[
        "PUBSUB_EMULATOR_HOST",
        "CLOUDSDK_API_ENDPOINT_OVERRIDES_PUBSUB",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_APPLICATION_CREDENTIALS_JSON",
        "GOOGLE_CREDENTIALS",
        "CLOUDSDK_AUTH_ACCESS_TOKEN",
        "GOOGLE_OAUTH_ACCESS_TOKEN",
    ];

    static PUBSUB_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    static METADATA_TOKENS: OnceLock<moka::sync::Cache<String, CachedMetadataToken>> =
        OnceLock::new();

    #[derive(Clone)]
    struct CachedMetadataToken {
        token: String,
        expires_at: chrono::DateTime<chrono::Utc>,
    }

    /// Body of every message this API publishes to a work topic.
    ///
    /// `payload` is byte-for-byte what the broker body used to be: the untagged
    /// inline job object or the `{"remote_url": …}` claim-check reference. The
    /// shape is identical to the Queue Storage envelope on purpose — one worker
    /// parser serves both clouds.
    #[derive(Serialize)]
    struct MessageEnvelope<'a, T: Serialize> {
        v: u8,
        job_id: &'a str,
        correlation_id: &'a str,
        workload: &'a str,
        payload: &'a T,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PublishResponse {
        #[serde(default)]
        message_ids: Vec<String>,
    }

    /// Byte cost of wrapping a payload in [`MessageEnvelope`], excluding the
    /// payload itself. `serde_json` emits the payload verbatim inside the
    /// envelope, so adding this to the serialized payload length is exact.
    fn envelope_overhead_bytes(job_id: &str, correlation_id: &str, workload: &str) -> usize {
        serde_json::to_vec(&MessageEnvelope {
            v: ENVELOPE_VERSION,
            job_id,
            correlation_id,
            workload,
            payload: &(),
        })
        .map(|bytes| bytes.len().saturating_sub("null".len()))
        .unwrap_or(0)
    }

    /// Whether a payload of `payload_bytes` must be staged to Cloud Storage
    /// instead of being inlined. The threshold applies to the enveloped
    /// message, not to the bare payload, because the envelope is what the
    /// broker retains.
    pub(crate) fn requires_claim_check(
        payload_bytes: usize,
        job_id: &str,
        correlation_id: &str,
        workload: &str,
    ) -> bool {
        payload_bytes.saturating_add(envelope_overhead_bytes(job_id, correlation_id, workload))
            >= CLAIM_CHECK_THRESHOLD_BYTES
    }

    pub(crate) async fn send_json<T: Serialize>(
        project_id: &str,
        topic: &str,
        job_id: &str,
        correlation_id: &str,
        workload: &str,
        payload: &T,
    ) -> Result<(), String> {
        reject_secret_credentials()?;
        let topic_resource = resolve_topic_resource(project_id, topic)?;
        validate_workload(workload)?;
        validate_identifier("job ID", job_id)?;
        validate_identifier("correlation ID", correlation_id)?;

        let body = serde_json::to_vec(&MessageEnvelope {
            v: ENVELOPE_VERSION,
            job_id,
            correlation_id,
            workload,
            payload,
        })
        .map_err(|error| format!("failed to serialize Pub/Sub message: {error}"))?;
        if body.len() > MAX_MESSAGE_DATA_BYTES {
            return Err(format!(
                "Pub/Sub message is {} bytes, above the {MAX_MESSAGE_DATA_BYTES} byte ceiling; bodies of {CLAIM_CHECK_THRESHOLD_BYTES} bytes or more should have been staged to Cloud Storage and sent as a claim-check reference",
                body.len()
            ));
        }

        let request_body = serde_json::json!({
            "messages": [{
                // `data` is proto3 `bytes` and the proto3 JSON mapping encodes
                // bytes as base64. This is the wire format rather than a
                // defensive choice, and it is why the size budget above is
                // computed against a 3:4 expansion of the request limit.
                "data": BASE64_STANDARD.encode(&body),
                // Attributes mirror the envelope's routing fields so a
                // subscription filter can route without decoding the body.
                // Filters are fixed when the subscription is created and can
                // only read attributes, so a message published without them
                // could never be matched by a filter added later. The body
                // stays authoritative: attributes are an unauthenticated side
                // channel capped at 1 KiB per value and are not part of the
                // versioned contract the worker validates.
                "attributes": {
                    "v": ENVELOPE_VERSION.to_string(),
                    "job_id": job_id,
                    "correlation_id": correlation_id,
                    "workload": workload
                }
                // No `orderingKey`. The SQS arm sets a message group per app
                // because FIFO queues demand one; the work topics are created
                // with message ordering disabled precisely so executions fan
                // out across workers. A key on an unordered topic buys nothing
                // today and would queue every execution of one app behind its
                // predecessor the moment ordering were switched on.
            }]
        });

        let token = fetch_metadata_token().await?;

        let response = pubsub_http_client()?
            .post(format!("{PUBSUB_ENDPOINT}/v1/{topic_resource}:publish"))
            .bearer_auth(&token)
            .json(&request_body)
            .send()
            .await
            .map_err(|error| format!("failed to publish Pub/Sub message: {error}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            let body = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY_BYTES)]);
            return Err(format!(
                "Pub/Sub rejected the publish to {topic_resource} with HTTP {status}: {body}"
            ));
        }

        // A success status carrying no message ID means the broker accepted the
        // request but persisted nothing. Reporting that as queued would hand
        // the caller a job ID no subscriber will ever see, and the run would
        // sit in `Running` until it aged out instead of failing at dispatch.
        let published: PublishResponse = response
            .json()
            .await
            .map_err(|error| format!("invalid Pub/Sub publish response: {error}"))?;
        if published.message_ids.is_empty() {
            return Err(format!(
                "Pub/Sub accepted the publish to {topic_resource} without returning a message ID"
            ));
        }

        Ok(())
    }

    fn pubsub_http_client() -> Result<&'static reqwest::Client, String> {
        if let Some(client) = PUBSUB_HTTP_CLIENT.get() {
            return Ok(client);
        }
        // `no_proxy` is load-bearing, not tidiness: this request carries a live
        // token for the workload's service account, and an ambient HTTPS_PROXY
        // would route it through a third party — one trusted enough to
        // terminate TLS would read the token outright. GCP deployments reach
        // the Google APIs over Private Google Access, never an egress proxy, so
        // no legitimate configuration is closed off here.
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
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

    /// Metadata authorities to try, in order: hostname first, link-local IP
    /// second.
    ///
    /// Mirrors object_store's `InstanceCredentialProvider` — including the
    /// `GCE_METADATA_*` overrides — so the token this module publishes with and
    /// the token object_store signs staging URLs with come from the same
    /// server. A deployment that redirected one and not the other would quietly
    /// run on two different identities, and the claim-check URL would be signed
    /// by a principal the topic grant knows nothing about. The IP fallback
    /// exists because metadata access must survive a pod with no working DNS;
    /// the hostname is unresolvable long before the address is unreachable.
    pub(super) fn metadata_authorities() -> [String; 2] {
        let non_empty = |value: String| {
            let value = value.trim().to_string();
            (!value.is_empty()).then_some(value)
        };

        let host = std::env::var("GCE_METADATA_HOST")
            .or_else(|_| std::env::var("GCE_METADATA_ROOT"))
            .ok()
            .and_then(non_empty)
            .unwrap_or_else(|| DEFAULT_METADATA_HOST.to_string());
        let ip = std::env::var("GCE_METADATA_IP")
            .ok()
            .and_then(non_empty)
            .unwrap_or_else(|| DEFAULT_METADATA_IP.to_string());

        [host, ip]
    }

    /// Fetch an OAuth2 access token for the workload's own service account from
    /// the GCE/Cloud Run metadata server.
    ///
    /// The token is requested narrowed to the Pub/Sub scope, falling back to the
    /// platform default when the metadata server refuses to downscope — see
    /// `NARROWED_TOKEN_QUERY`. Narrowing is a second line of defence only: the
    /// restriction that always holds is IAM, where the API's service account
    /// holds `roles/pubsub.publisher` on the work topics and nothing else, which
    /// the broker enforces server-side on every publish.
    ///
    /// This duplicates the token source in `credentials::gcp_credentials`
    /// deliberately. `pubsub` is a transport feature and must compile without
    /// `gcp`, which is the scoped-credential surface; sharing the helper would
    /// couple a deployment that only publishes jobs to the RS256 assertion
    /// signing and STS downscoping it never performs. The two caches are
    /// independent, so a process running both pays at most one extra metadata
    /// round trip every ten minutes.
    async fn fetch_metadata_token() -> Result<String, String> {
        let [host, ip] = metadata_authorities();

        let cache = METADATA_TOKENS.get_or_init(|| {
            moka::sync::Cache::builder()
                .max_capacity(4)
                .time_to_live(Duration::from_secs(METADATA_TOKEN_CACHE_TTL_SECONDS))
                .build()
        });

        // The moka TTL is a ceiling, not the truth — the metadata server decides
        // the real lifetime. Re-checking the advertised expiry keeps a
        // nearly-dead token out of a publish. A freshly fetched short token is
        // still used: the metadata server refreshes on its own schedule and
        // rejecting one here would turn a narrow window into a hard outage.
        if let Some(cached) = cache.get(&host)
            && cached.expires_at - chrono::Utc::now()
                > chrono::Duration::seconds(METADATA_TOKEN_MIN_LIFETIME_SECONDS)
        {
            return Ok(cached.token);
        }

        // `no_proxy` again, for the same reason as the publish client, plus one
        // more: the metadata server is link-local and unroutable, so there is
        // never a legitimate proxy for it. HTTPS is likewise absent by design —
        // the endpoint is plain HTTP on an address only the hypervisor can
        // answer for, which is why `https_only` is not set here as it is above.
        //
        // Redirects are refused because the only correct responder for this
        // request is the link-local address itself: a 3xx pointing elsewhere is
        // either a misconfiguration or an attempt to hand this process a token
        // from a server it does not trust.
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(METADATA_CONNECT_TIMEOUT_SECONDS))
            .timeout(Duration::from_secs(METADATA_REQUEST_TIMEOUT_SECONDS))
            .build()
            .map_err(|error| format!("failed to construct GCP metadata client: {error}"))?;

        #[derive(Deserialize)]
        struct MetadataTokenResponse {
            access_token: String,
            expires_in: i64,
        }

        let mut last_error: Option<String> = None;
        for authority in [host.as_str(), ip.as_str()] {
            for query in [NARROWED_TOKEN_QUERY, UNNARROWED_TOKEN_QUERY] {
                let narrowed = !query.is_empty();

                // `Metadata-Flavor: Google` is the SSRF guard, not a formality:
                // the metadata server rejects any request that omits it, so a
                // confused proxy or a redirect-following client cannot be steered
                // into reading the token on an attacker's behalf. The response
                // echo is only logged, not required — see the check below.
                let response = client
                    .get(format!("http://{}{}", authority, METADATA_TOKEN_PATH))
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
                        // The authority did not answer at all, so the fallback
                        // query would fail identically. Try the next authority.
                        break;
                    }
                };

                let status = response.status();
                let flavor_echoed = response
                    .headers()
                    .get(METADATA_FLAVOR_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.eq_ignore_ascii_case(METADATA_FLAVOR_VALUE));

                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    last_error = Some(format!(
                        "GCP metadata server at {authority} returned HTTP {status}: {body}"
                    ));
                    if narrowed {
                        tracing::warn!(
                            authority,
                            %status,
                            "GCP metadata server rejected the scope-narrowed token request; retrying without scope enforcement"
                        );
                        continue;
                    }
                    break;
                }

                // Warned about, never enforced — see the matching note in
                // `credentials::gcp_credentials`. Google documents this header as
                // a request requirement only; `google-auth` checks it in `ping()`
                // and not on token fetches, object_store never checks it, and GKE
                // has been reported serving token responses without it.
                if !flavor_echoed {
                    tracing::warn!(
                        authority,
                        "GCP metadata token response omitted {METADATA_FLAVOR_HEADER}: {METADATA_FLAVOR_VALUE}"
                    );
                }

                // A malformed body falls through to the next authority rather
                // than aborting: the hostname answering with junk is exactly the
                // broken-DNS case the link-local address exists to survive.
                let token: MetadataTokenResponse = match response.json().await {
                    Ok(token) => token,
                    Err(error) => {
                        last_error = Some(format!(
                            "invalid GCP metadata token response from {authority}: {error}"
                        ));
                        break;
                    }
                };

                // The metadata token response carries no `scope` field, so a
                // platform that ignores the narrowing rather than rejecting it is
                // indistinguishable from one that honoured it. This warning marks
                // the case we can actually detect: the request we sent was wide.
                if !narrowed {
                    tracing::warn!(
                        authority,
                        "publishing with an unnarrowed GCP metadata token; it carries the runtime service account's full scope"
                    );
                }

                // Clamped because `chrono::Duration::seconds` panics on an absurd
                // input and this runs inside a request handler. Clamping low
                // degrades to "never serve from cache", which is correct rather
                // than merely safe.
                let lifetime = chrono::Duration::seconds(token.expires_in.clamp(0, 24 * 60 * 60));
                cache.insert(
                    host.clone(),
                    CachedMetadataToken {
                        token: token.access_token.clone(),
                        expires_at: chrono::Utc::now() + lifetime,
                    },
                );

                return Ok(token.access_token);
            }
        }

        Err(last_error.unwrap_or_else(|| {
            format!(
                "no GCP credentials available for Pub/Sub: the metadata server was unreachable at \
                 {host} and {ip}. Bind a runtime service account with roles/pubsub.publisher to \
                 the Cloud Run revision."
            )
        }))
    }

    fn reject_secret_credentials() -> Result<(), String> {
        for variable in FORBIDDEN_CREDENTIAL_SETTINGS {
            if std::env::var(variable)
                .ok()
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(format!(
                    "{variable} is forbidden for Pub/Sub; use the runtime service account bound to the workload"
                ));
            }
        }
        Ok(())
    }

    /// Resolve the configured topic into the `projects/<p>/topics/<t>` resource
    /// name the publish URL is built from.
    ///
    /// The deployment emits the fully qualified form, but a bare topic ID is
    /// accepted so a hand-assembled environment does not have to repeat the
    /// project. When the topic is qualified the project is compared rather than
    /// ignored: a topic name left behind by another project would otherwise
    /// publish jobs the local workers never subscribe to, and the run would sit
    /// in `Running` until it aged out instead of failing at dispatch.
    fn resolve_topic_resource(project_id: &str, topic: &str) -> Result<String, String> {
        let project_id = project_id.trim();
        let topic = topic.trim();

        if let Some(rest) = topic.strip_prefix("projects/") {
            let (topic_project, topic_id) = rest.split_once("/topics/").ok_or_else(|| {
                format!(
                    "Pub/Sub topic '{topic}' is not a projects/<project>/topics/<topic> resource name"
                )
            })?;
            validate_project_id(topic_project)?;
            validate_topic_id(topic_id)?;
            if !project_id.is_empty() && project_id != topic_project {
                return Err(format!(
                    "Pub/Sub topic names project '{topic_project}' but the workload is configured for project '{project_id}'"
                ));
            }
            return Ok(format!("projects/{topic_project}/topics/{topic_id}"));
        }

        validate_project_id(project_id)?;
        validate_topic_id(topic)?;
        Ok(format!("projects/{project_id}/topics/{topic}"))
    }

    fn validate_project_id(value: &str) -> Result<(), String> {
        let bytes = value.as_bytes();
        let valid = (6..=30).contains(&value.len())
            && bytes.first().is_some_and(u8::is_ascii_lowercase)
            && bytes
                .last()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-');
        if !valid {
            return Err(format!(
                "GCP project ID '{value}' is invalid; expected 6-30 characters of [a-z0-9-] starting with a letter"
            ));
        }
        Ok(())
    }

    fn validate_topic_id(value: &str) -> Result<(), String> {
        // Pub/Sub also accepts `%` and `+` in a topic ID, and both change
        // meaning inside a URL path segment — `%` opens an escape sequence, so
        // the resource actually published to would differ from the one
        // configured. The deployment never generates either, so they are
        // rejected here instead of being percent-encoded on the way out.
        let valid = (3..=255).contains(&value.len())
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
            // `goog` is a reserved prefix Pub/Sub refuses to create, so a topic
            // carrying it cannot be one this deployment owns.
            && !value.to_ascii_lowercase().starts_with("goog");
        if !valid {
            return Err(format!("Pub/Sub topic ID '{value}' is invalid"));
        }
        Ok(())
    }

    fn validate_workload(value: &str) -> Result<(), String> {
        if !SUPPORTED_WORKLOADS.contains(&value) {
            return Err(format!("unsupported Pub/Sub workload '{value}'"));
        }
        Ok(())
    }

    fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(format!("Pub/Sub message {kind} is invalid"));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bare_topic_ids_resolve_against_the_configured_project() {
            assert_eq!(
                resolve_topic_resource("flow-like-dev", "flow-like-execution").unwrap(),
                "projects/flow-like-dev/topics/flow-like-execution"
            );
        }

        #[test]
        fn qualified_topic_names_are_accepted_unchanged() {
            assert_eq!(
                resolve_topic_resource(
                    "flow-like-dev",
                    "projects/flow-like-dev/topics/flow-like-execution"
                )
                .unwrap(),
                "projects/flow-like-dev/topics/flow-like-execution"
            );
        }

        #[test]
        fn a_topic_from_another_project_is_refused_before_publishing() {
            assert!(
                resolve_topic_resource(
                    "flow-like-dev",
                    "projects/flow-like-prod/topics/flow-like-execution"
                )
                .is_err()
            );
        }

        #[test]
        fn topic_names_are_restricted_to_deployment_generated_names() {
            assert!(validate_topic_id("flow-like-execution").is_ok());
            assert!(validate_topic_id("execution_v2.retry~1").is_ok());
            // Legal Pub/Sub names that are ambiguous inside a URL path.
            assert!(validate_topic_id("exec%2Fution").is_err());
            assert!(validate_topic_id("exec+ution").is_err());
            // Reserved prefix, path separator, leading digit, too short.
            assert!(validate_topic_id("goog-execution").is_err());
            assert!(validate_topic_id("execution/jobs").is_err());
            assert!(validate_topic_id("1execution").is_err());
            assert!(validate_topic_id("ex").is_err());
        }

        #[test]
        fn project_ids_follow_the_documented_google_shape() {
            assert!(validate_project_id("flow-like-dev").is_ok());
            assert!(validate_project_id("Flow-Like-Dev").is_err());
            assert!(validate_project_id("flowlike-").is_err());
            assert!(validate_project_id("1flowlike").is_err());
            assert!(validate_project_id("flow").is_err());
            assert!(validate_project_id("a".repeat(31).as_str()).is_err());
        }

        #[test]
        fn workloads_are_restricted_to_the_deployed_worker_set() {
            assert!(validate_workload(WORKLOAD_EXECUTION).is_ok());
            assert!(validate_workload(WORKLOAD_COMPILATION).is_ok());
            assert!(validate_workload(WORKLOAD_FILE_TRACKING).is_ok());
            assert!(validate_workload(WORKLOAD_MEDIA_TRANSFORMATION).is_ok());
            assert!(validate_workload("file-tracker").is_err());
            assert!(validate_workload("").is_err());
        }

        #[test]
        fn envelope_overhead_matches_the_serialized_envelope() {
            let payload = serde_json::json!({ "remote_url": "https://example.invalid/object" });
            let payload_bytes = serde_json::to_vec(&payload).expect("payload serializes");
            let envelope = serde_json::to_vec(&MessageEnvelope {
                v: ENVELOPE_VERSION,
                job_id: "job-1",
                correlation_id: "app-1",
                workload: WORKLOAD_EXECUTION,
                payload: &payload,
            })
            .expect("envelope serializes");

            assert_eq!(
                envelope.len(),
                payload_bytes.len() + envelope_overhead_bytes("job-1", "app-1", WORKLOAD_EXECUTION)
            );
        }

        #[test]
        fn claim_check_triggers_far_below_the_hard_wire_limit() {
            let overhead = envelope_overhead_bytes("job-1", "app-1", WORKLOAD_EXECUTION);
            assert!(!requires_claim_check(
                CLAIM_CHECK_THRESHOLD_BYTES - overhead - 1,
                "job-1",
                "app-1",
                WORKLOAD_EXECUTION
            ));
            assert!(requires_claim_check(
                CLAIM_CHECK_THRESHOLD_BYTES - overhead,
                "job-1",
                "app-1",
                WORKLOAD_EXECUTION
            ));
            // Every inlined message still leaves room under the publish limit.
            assert!(CLAIM_CHECK_THRESHOLD_BYTES < MAX_MESSAGE_DATA_BYTES);
            assert_eq!(MAX_MESSAGE_DATA_BYTES, 10_000_000 * 3 / 4);
        }

        #[test]
        fn claim_check_url_stays_within_the_gcs_signing_ceiling() {
            // GCS refuses to mint a V4 signature valid for more than seven days,
            // so a longer TTL would fail at signing time rather than at fetch.
            assert!(CLAIM_CHECK_URL_TTL.as_secs() <= 7 * 86_400);
        }
    }
}

/// Google ID tokens for the executor's IAM front door on synchronous dispatch.
///
/// The executor Cloud Run service admits only the API's service account as
/// `roles/run.invoker`, so Cloud Run's front-end rejects any request that does
/// not carry a Google-signed ID token whose `aud` is the service URL. Like the
/// `pubsub` publisher above, the only identity this module will present is the
/// one the metadata server hands out for the running revision — there is no
/// key file path and no endpoint override — which is also why it rides the
/// same `pubsub` feature: that flag is the metadata-server dispatch surface of
/// this file, not the Pub/Sub wire protocol.
#[cfg(feature = "pubsub")]
pub(crate) mod gcp_id_token {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL_SAFE};
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::pubsub;

    /// Metadata-server endpoint that mints an ID token for the service account
    /// bound to the running revision, signed by Google for the audience passed
    /// in the query string. `format=full` is requested so the token carries
    /// the claims Cloud Run's front-end validates.
    const METADATA_IDENTITY_PATH: &str =
        "/computeMetadata/v1/instance/service-accounts/default/identity";

    /// A cached token is only reused while this much life remains, for the
    /// same reason the Pub/Sub token keeps a margin: a request authenticated
    /// just before expiry can still be rejected mid-flight, and here that
    /// surfaces as a spurious 401 on a dispatch that was authorized moments
    /// earlier.
    const ID_TOKEN_MIN_LIFETIME_SECONDS: i64 = 5 * 60;
    /// Ceiling on cache residency. Google ID tokens live one hour; 45 minutes
    /// keeps a token for most of that life while the min-lifetime check above,
    /// driven by the `exp` the token actually advertises, remains the real
    /// expiry authority.
    const ID_TOKEN_CACHE_TTL_SECONDS: u64 = 45 * 60;

    static ID_TOKENS: OnceLock<moka::sync::Cache<String, CachedIdToken>> = OnceLock::new();

    #[derive(Clone)]
    struct CachedIdToken {
        token: String,
        expires_at: chrono::DateTime<chrono::Utc>,
    }

    /// Derive the token audience from the configured executor URL.
    ///
    /// Cloud Run validates `aud` against the service URL, which is always a
    /// bare origin, so `EXECUTOR_URL` is used exactly as configured when it
    /// carries no path — minus a lone trailing slash, which URL parsing would
    /// report as a path the operator never meant — and is reduced to
    /// `scheme://host[:port]` when it does (the deployment appends `/execute`
    /// paths at request time, but a hand-assembled environment may have baked
    /// one in).
    pub(crate) fn audience_from_executor_url(executor_url: &str) -> Result<String, String> {
        let trimmed = executor_url.trim();
        let parsed = reqwest::Url::parse(trimmed)
            .map_err(|error| format!("EXECUTOR_URL '{trimmed}' is not a valid URL: {error}"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!(
                "EXECUTOR_URL '{trimmed}' must be an http(s) URL to derive an ID-token audience"
            ));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| format!("EXECUTOR_URL '{trimmed}' carries no host"))?;

        if matches!(parsed.path(), "" | "/")
            && parsed.query().is_none()
            && parsed.fragment().is_none()
        {
            return Ok(trimmed.trim_end_matches('/').to_string());
        }

        // `Url::port()` is `None` for a scheme-default port, so the origin
        // only names a port the operator wrote explicitly — exactly what a
        // non-Cloud-Run executor behind IAM-style auth would have configured
        // as its audience.
        let mut audience = format!("{}://{}", parsed.scheme(), host);
        if let Some(port) = parsed.port() {
            audience.push_str(&format!(":{port}"));
        }
        Ok(audience)
    }

    /// Fetch a Google-signed ID token for `audience` from the metadata server,
    /// serving from cache while the token's own `exp` leaves enough life.
    ///
    /// An unreachable metadata server is an error the caller must surface, not
    /// a reason to dispatch without the header: the executor's front door
    /// would refuse the request anyway, and the 403 it returns names neither
    /// the missing token nor the reason it could not be minted.
    pub(crate) async fn fetch_id_token(audience: &str) -> Result<String, String> {
        let [host, ip] = pubsub::metadata_authorities();

        let cache = ID_TOKENS.get_or_init(|| {
            moka::sync::Cache::builder()
                .max_capacity(4)
                .time_to_live(Duration::from_secs(ID_TOKEN_CACHE_TTL_SECONDS))
                .build()
        });

        if let Some(cached) = cache.get(audience)
            && cached.expires_at - chrono::Utc::now()
                > chrono::Duration::seconds(ID_TOKEN_MIN_LIFETIME_SECONDS)
        {
            return Ok(cached.token);
        }

        // `no_proxy` and plain HTTP for the same reasons the Pub/Sub token
        // fetch gives: the metadata server is link-local, unroutable and
        // answered only by the hypervisor.
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(
                pubsub::METADATA_CONNECT_TIMEOUT_SECONDS,
            ))
            .timeout(Duration::from_secs(
                pubsub::METADATA_REQUEST_TIMEOUT_SECONDS,
            ))
            .build()
            .map_err(|error| format!("failed to construct GCP metadata client: {error}"))?;

        let mut last_error: Option<String> = None;
        for authority in [host.as_str(), ip.as_str()] {
            let response = client
                .get(format!("http://{authority}{METADATA_IDENTITY_PATH}"))
                .query(&[("audience", audience), ("format", "full")])
                .header("Metadata-Flavor", "Google")
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    last_error = Some(format!(
                        "GCP metadata identity request to {authority} failed: {error}"
                    ));
                    continue;
                }
            };

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                last_error = Some(format!(
                    "GCP metadata server at {authority} returned HTTP {status}: {body}"
                ));
                continue;
            }

            // The identity endpoint returns the JWT as a bare text body.
            let token = response
                .text()
                .await
                .map_err(|error| {
                    format!("invalid GCP metadata identity response from {authority}: {error}")
                })?
                .trim()
                .to_string();
            if token.is_empty() {
                last_error = Some(format!(
                    "GCP metadata server at {authority} returned an empty ID token"
                ));
                continue;
            }

            // An unparseable `exp` degrades to "never serve from cache" — the
            // token is still presented once, because Cloud Run is the party
            // that verifies it, and refusing here would turn a decoding quirk
            // into a dispatch outage.
            let expires_at = decode_jwt_exp(&token).unwrap_or_else(chrono::Utc::now);
            cache.insert(
                audience.to_string(),
                CachedIdToken {
                    token: token.clone(),
                    expires_at,
                },
            );

            return Ok(token);
        }

        Err(last_error.unwrap_or_else(|| {
            format!(
                "no Google ID token available for the executor: the metadata server was \
                 unreachable at {host} and {ip}. EXECUTOR_AUTH=gcp_id_token only works where a \
                 runtime service account with roles/run.invoker on the executor service is bound \
                 to the workload."
            )
        }))
    }

    /// Expiry claim of the minted token. Only `exp` is read and the signature
    /// is deliberately not verified — this process is the token's client, not
    /// its audience; Cloud Run performs the real validation.
    fn decode_jwt_exp(jwt: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        let payload = jwt.split('.').nth(1)?;
        let bytes = BASE64_URL_SAFE.decode(payload).ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        chrono::DateTime::from_timestamp(claims.get("exp")?.as_i64()?, 0)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_bare_service_url_is_used_exactly() {
            assert_eq!(
                audience_from_executor_url("https://executor-abc123-uc.a.run.app").unwrap(),
                "https://executor-abc123-uc.a.run.app"
            );
        }

        #[test]
        fn a_lone_trailing_slash_is_not_a_path() {
            assert_eq!(
                audience_from_executor_url("https://executor-abc123-uc.a.run.app/").unwrap(),
                "https://executor-abc123-uc.a.run.app"
            );
        }

        #[test]
        fn a_path_is_reduced_to_the_origin() {
            assert_eq!(
                audience_from_executor_url("https://executor-abc123-uc.a.run.app/execute").unwrap(),
                "https://executor-abc123-uc.a.run.app"
            );
        }

        #[test]
        fn a_query_is_reduced_to_the_origin() {
            assert_eq!(
                audience_from_executor_url("https://executor-abc123-uc.a.run.app?region=eu")
                    .unwrap(),
                "https://executor-abc123-uc.a.run.app"
            );
        }

        #[test]
        fn an_explicit_non_default_port_survives_the_reduction() {
            assert_eq!(
                audience_from_executor_url("http://executor.internal:8080/execute").unwrap(),
                "http://executor.internal:8080"
            );
        }

        #[test]
        fn a_scheme_default_port_is_dropped_by_the_reduction() {
            assert_eq!(
                audience_from_executor_url("https://executor-abc123-uc.a.run.app:443/execute")
                    .unwrap(),
                "https://executor-abc123-uc.a.run.app"
            );
        }

        #[test]
        fn non_http_and_hostless_urls_are_refused() {
            assert!(audience_from_executor_url("ftp://executor").is_err());
            assert!(audience_from_executor_url("unix:///var/run/executor.sock").is_err());
            assert!(audience_from_executor_url("not a url").is_err());
            assert!(audience_from_executor_url("").is_err());
        }
    }
}

/// Fetch a profile for a user from the database and convert it
/// to a JSON value matching the core `Profile` struct format for the executor.
///
/// Resolution order:
/// 1. If `profile_id` is provided, fetch that specific profile (must belong to user)
/// 2. Otherwise find the first profile whose `apps` list contains the given `app_id`
/// 3. Fallback to the first profile for the user
///
/// The user's private custom bits are inlined as `custom_bits`. Pass
/// `include_secrets = true` only when the resulting JSON stays inside an
/// ephemeral dispatch (encrypted payload storage / in-memory); persisted
/// copies (event rows) must use `false` and re-hydrate at trigger time via
/// [`hydrate_profile_custom_bit_secrets`].
pub async fn fetch_profile_for_dispatch(
    state: &crate::state::AppState,
    user_id: &str,
    profile_id: Option<&str>,
    app_id: &str,
    include_secrets: bool,
) -> Option<serde_json::Value> {
    use crate::entity::profile;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = &state.db;

    let model = if let Some(pid) = profile_id {
        profile::Entity::find()
            .filter(
                profile::Column::Id
                    .eq(pid)
                    .and(profile::Column::UserId.eq(user_id))
                    .and(profile::Column::DeletedAt.is_null()),
            )
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
            .filter(
                profile::Column::UserId
                    .eq(user_id)
                    .and(profile::Column::DeletedAt.is_null()),
            )
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

    let profile_bit_ids = model.bit_ids.clone().unwrap_or_default();
    let custom_bits = crate::routes::user::bits::load_custom_bits_for_profile(
        state,
        user_id,
        &profile_bit_ids,
        include_secrets,
    )
    .await
    .unwrap_or_else(|err| {
        tracing::warn!(user_id = %user_id, "Failed to load custom bits for dispatch: {err:?}");
        vec![]
    });
    let custom_bits = serde_json::to_value(custom_bits).unwrap_or_else(|_| serde_json::json!([]));

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
        "custom_bits": custom_bits,
        "settings": model.settings.unwrap_or_else(|| serde_json::json!({"connection_mode": "simplebezier"})),
        "updated": model.updated_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        "created": model.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
    }))
}

/// Re-hydrates decrypted provider secrets into the `custom_bits` of a
/// persisted profile JSON (event/sink rows store profiles WITHOUT secrets).
/// Bits are looked up by id — the ids were placed server-side from the owner's
/// profile at setup time, so they act as unforgeable references.
pub async fn hydrate_profile_custom_bit_secrets(
    state: &crate::state::AppState,
    profile_json: &mut serde_json::Value,
) {
    use crate::entity::user_bit;
    use sea_orm::EntityTrait;

    let Some(custom_bits) = profile_json
        .get_mut("custom_bits")
        .and_then(|bits| bits.as_array_mut())
    else {
        return;
    };

    for bit in custom_bits {
        let Some(bit_id) = bit.get("id").and_then(|id| id.as_str()) else {
            continue;
        };

        let row = match user_bit::Entity::find_by_id(bit_id).one(&state.db).await {
            Ok(Some(row)) => row,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(bit_id = %bit_id, "Failed to load custom bit for hydration: {err:?}");
                continue;
            }
        };

        let hydrated = crate::routes::user::bits::user_bit_to_core(row, state, true);
        if let Ok(hydrated) = serde_json::to_value(&hydrated) {
            *bit = hydrated;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The gate is exercised through the pure function rather than the
    // environment: process-wide env mutation races the rest of the parallel
    // test binary, and `attach_executor_iam_auth` adds nothing beyond the
    // `std::env::var` read this function receives verbatim.
    #[test]
    fn executor_auth_gate_matches_only_gcp_id_token() {
        assert!(executor_auth_requires_gcp_id_token(Some("gcp_id_token")));
        assert!(executor_auth_requires_gcp_id_token(Some("GCP_ID_TOKEN")));
        assert!(executor_auth_requires_gcp_id_token(Some(
            "  gcp_id_token\n"
        )));
        assert!(!executor_auth_requires_gcp_id_token(Some("")));
        assert!(!executor_auth_requires_gcp_id_token(Some("none")));
        assert!(!executor_auth_requires_gcp_id_token(Some("backend_jwt")));
        assert!(!executor_auth_requires_gcp_id_token(Some("gcp_id_tokens")));
        assert!(!executor_auth_requires_gcp_id_token(None));
    }

    #[test]
    fn tenant_isolation_flag_accepts_only_documented_values() {
        for enabling in [
            "sub", "SUB", "  sub\n", "user", "user_id", "true", "1", "on", "enabled",
        ] {
            assert_eq!(
                lambda_tenant_isolation_enabled(Some(enabling)),
                Ok(true),
                "{enabling:?} should enable tenant isolation"
            );
        }

        for disabling in ["", "   ", "off", "OFF", "false", "0", "none", "disabled"] {
            assert_eq!(
                lambda_tenant_isolation_enabled(Some(disabling)),
                Ok(false),
                "{disabling:?} should disable tenant isolation"
            );
        }

        assert_eq!(lambda_tenant_isolation_enabled(None), Ok(false));

        // A typo must not read as "disabled": silently dropping the tenant id
        // would leave the operator believing runs are isolated when they share
        // execution environments.
        for typo in ["subs", "per_tenant", "yes", "sub sub"] {
            assert!(
                lambda_tenant_isolation_enabled(Some(typo)).is_err(),
                "{typo:?} should be refused rather than silently disabling"
            );
        }
    }

    /// Every character AWS accepts in `X-Amz-Tenant-Id`, per the `Invoke`
    /// constraint `[a-zA-Z0-9\._:\/=+\-@ ]+` with a length of 1..=256.
    fn is_aws_tenant_id(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 256
            && value.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '.' | '_' | ':' | '/' | '=' | '+' | '-' | '@' | ' ')
            })
    }

    #[test]
    fn tenant_ids_are_aws_shaped_for_every_subject_shape() {
        let long_subject = "x".repeat(300);
        let subjects = [
            "",
            "local",
            "sink:abc123",
            "inbound:xyz789",
            "5f8d0d55-b1c4-4f3a-9c2e-0a1b2c3d4e5f",
            // Federated subjects carry `|`, which AWS's tenant charset rejects.
            "auth0|123",
            "google-oauth2|1234567890",
            "user@example.com",
            "Ünïcödé-サブジェクト",
            long_subject.as_str(),
        ];

        for subject in subjects {
            let tenant = lambda_tenant_id(subject);
            assert!(
                is_aws_tenant_id(&tenant),
                "{subject:?} produced a tenant id AWS would reject: {tenant:?}"
            );
            assert_eq!(tenant.len(), 1 + LAMBDA_TENANT_ID_HEX_CHARS);
            assert_eq!(
                tenant,
                lambda_tenant_id(subject),
                "derivation must be stable across calls"
            );
        }
    }

    #[test]
    fn distinct_subjects_never_share_an_execution_environment() {
        let collision_bait = [
            "auth0|123",
            "auth0:123",
            "auth0_123",
            "auth0123",
            // AWS does not document whether tenant ids are case-folded, so the
            // derivation emits lowercase hex and distinguishes these itself.
            "Alice",
            "alice",
            "",
            "user",
        ];

        let mut seen = std::collections::HashSet::new();
        for subject in collision_bait {
            assert!(
                seen.insert(lambda_tenant_id(subject)),
                "{subject:?} collided with an earlier subject"
            );
        }
    }

    #[test]
    fn tenant_ids_are_domain_separated_from_storage_path_digests() {
        // `storage_path_segment` appends the first 12 hex characters of a bare
        // `blake3::hash(sub)`. Sharing that digest would let a storage prefix
        // confirm a tenant id for the same subject.
        let subject = "auth0|123";
        let bare = blake3::hash(subject.as_bytes()).to_hex();
        let tenant = lambda_tenant_id(subject);
        assert!(
            !tenant[1..].starts_with(&bare[..12]),
            "tenant id must not reuse the storage-path digest"
        );
    }
}
