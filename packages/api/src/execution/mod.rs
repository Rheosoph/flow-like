//! Execution module for runtime authentication and job management.
//!
//! This module provides JWT-based authentication for execution environments
//! (Kubernetes, Docker Compose, Lambda, etc.) to securely communicate with the API.

mod channel_jwt;
pub mod compiled_artifacts;
mod dispatch;
mod jwt;
pub mod payload_storage;
pub mod queue;
pub mod rejection;
pub mod run_sweeper;
mod sse_proxy;
pub mod state;
pub mod wasm_resolve;

pub use crate::backend_jwt::TokenType;
pub use channel_jwt::{
    ChannelClaims, ChannelJwtError, ChannelJwtParams, sign_channel_responder,
    verify_channel_responder,
};
pub use dispatch::{
    ArtifactEnsurer, ByteStream, DispatchConfig, DispatchError, DispatchRequest, DispatchResponse,
    Dispatcher, ExecutionBackend, StreamChunk, fetch_profile_for_dispatch,
    hydrate_profile_custom_bit_secrets,
};
pub use jwt::{
    ExecutionClaims, ExecutionJwk, ExecutionJwks, ExecutionJwtError, ExecutionJwtParams,
    get_jwks as get_execution_jwks, is_configured as is_jwt_configured, sign as sign_execution_jwt,
    verify as verify_execution_jwt, verify_user as verify_user_jwt,
};
#[cfg(feature = "redis")]
pub use queue::QueueWorker;
pub use queue::{OAuthTokenInput, QueueConfig, QueueError, QueuedJob};
pub use run_sweeper::{RunSweeperConfig, spawn_run_sweeper};
pub(crate) use sse_proxy::completed_run_status;
pub use sse_proxy::{
    collect_generic_result, collect_generic_result_bytes, proxy_sse_response,
    update_run_on_completion,
};
pub use state::{
    CreateEventInput, CreateRunInput, EventQuery, ExecutionEventRecord, ExecutionRunRecord,
    ExecutionStateStore, RunMode, RunStatus, StateBackend, StateStoreConfig, StateStoreError,
    UpdateRunInput, create_state_store,
};
pub use wasm_resolve::resolve_wasm_packages;
