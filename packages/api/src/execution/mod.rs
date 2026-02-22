//! Execution module for runtime authentication and job management.
//!
//! This module provides JWT-based authentication for execution environments
//! (Kubernetes, Docker Compose, Lambda, etc.) to securely communicate with the API.

mod dispatch;
mod interaction_jwt;
mod jwt;
pub mod payload_storage;
pub mod queue;
mod sse_proxy;
pub mod state;
pub mod wasm_resolve;

pub use crate::backend_jwt::TokenType;
pub use dispatch::{
    ByteStream, DispatchConfig, DispatchError, DispatchRequest, DispatchResponse, Dispatcher,
    ExecutionBackend, StreamChunk, fetch_profile_for_dispatch,
};
pub use interaction_jwt::{
    InteractionClaims, InteractionJwtError, InteractionJwtParams,
    sign_responder as sign_interaction_responder_jwt,
    verify_responder as verify_interaction_responder_jwt,
};
pub use jwt::{
    ExecutionClaims, ExecutionJwk, ExecutionJwks, ExecutionJwtError, ExecutionJwtParams,
    get_jwks as get_execution_jwks, is_configured as is_jwt_configured, sign as sign_execution_jwt,
    verify as verify_execution_jwt, verify_user as verify_user_jwt,
};
#[cfg(feature = "redis")]
pub use queue::QueueWorker;
pub use queue::{OAuthTokenInput, QueueConfig, QueueError, QueuedJob};
pub use sse_proxy::proxy_sse_response;
pub use wasm_resolve::resolve_wasm_packages;
pub use state::{
    CreateEventInput, CreateRunInput, EventQuery, ExecutionEventRecord, ExecutionRunRecord,
    ExecutionStateStore, RunMode, RunStatus, StateBackend, StateStoreConfig, StateStoreError,
    UpdateRunInput, create_state_store,
};
