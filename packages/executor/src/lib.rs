//! Flow-Like Executor
//!
//! Environment-agnostic execution runtime that works across:
//! - AWS Lambda (with streaming responses)
//! - Azure Functions
//! - Kubernetes pods
//! - Docker Compose containers
//! - Any other execution environment
//!
//! ## Execution Modes
//!
//! ### Callback Mode
//! The executor receives an `ExecutionRequest` with a JWT containing a callback URL.
//! Events are batched and sent to the callback URL during execution.
//! Good for queue-based/decoupled execution.
//!
//! ### Streaming Mode
//! Events are streamed directly back to the caller via NDJSON or SSE.
//! Perfect for Lambda streaming responses or direct API calls.
//!
//! ## Usage
//!
//! ```rust,ignore
//! // Use the Axum router for HTTP endpoints
//! use flow_like_executor::{executor_router, ExecutorState};
//!
//! let state = ExecutorState::from_env();
//! let app = executor_router(state);
//! ```

pub mod channel;
pub mod config;
pub mod error;
pub mod execute;
pub mod jwt;
pub mod resolve;
pub mod router;
pub mod streaming;
pub mod types;
pub mod wasm_loader;

pub use config::ExecutorConfig;
pub use error::ExecutorError;
pub use execute::{execute, report_queue_failure};
pub use flow_like_types::OAuthTokenInput;
pub use resolve::{
    fetch_bounded, resolve_payload, resolve_payload_from_str, ResolveError,
    MAX_REMOTE_PAYLOAD_BYTES,
};
pub use router::{executor_router, ExecutorState};
pub use streaming::{execute_streaming, ExecutionStream, StreamEvent};
pub use types::{BoardVersion, ExecutionEvent, ExecutionRequest, ExecutionResult, ExecutionStatus};
