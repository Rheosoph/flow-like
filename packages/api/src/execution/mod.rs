//! Execution module for runtime authentication and job management.
//!
//! This module provides JWT-based authentication for execution environments
//! (Kubernetes, Docker Compose, Lambda, etc.) to securely communicate with the API.

mod channel_jwt;
pub mod compiled_artifacts;
mod dispatch;
mod jwt;
mod page_action_jwt;
mod page_action_sealer;
pub mod payload_storage;
pub mod queue;
pub mod regression;
pub mod rejection;
pub mod run_sweeper;
mod sse_proxy;
pub mod state;
pub mod variant;
pub mod wasm_node_stubs;
pub mod wasm_resolve;

pub use crate::backend_jwt::TokenType;
pub use channel_jwt::{
    ChannelClaims, ChannelJwtError, ChannelJwtParams, sign_channel_responder,
    verify_channel_responder,
};
pub use dispatch::{
    ArtifactEnsurer, ByteStream, DispatchConfig, DispatchError, DispatchRequest, DispatchResponse,
    DispatchTrigger, Dispatcher, ExecutionBackend, StreamChunk, fetch_profile_for_dispatch,
    hydrate_profile_custom_bit_secrets,
};
pub use jwt::{
    ExecutionClaims, ExecutionJwk, ExecutionJwks, ExecutionJwtError, ExecutionJwtParams,
    PageExecutionJwtContext, get_jwks as get_execution_jwks, is_configured as is_jwt_configured,
    sign as sign_execution_jwt, sign_with_page_context as sign_execution_jwt_with_page_context,
    verify as verify_execution_jwt, verify_user as verify_user_jwt,
};
pub use page_action_jwt::{
    MAX_PAGE_ACTION_TTL_SECONDS, PAGE_ACTION_CAPABILITY_VERSION, PageActionClaims,
    PageActionJwtError, PageActionJwtParams, sign_page_action_capability,
    verify_page_action_capability,
};
pub use page_action_sealer::{
    DYNAMIC_PAGE_ACTION_ID_PREFIX, PageActionSealingContext, PageActionSealingReport,
};
#[cfg(feature = "redis")]
pub use queue::QueueWorker;
pub use queue::{OAuthTokenInput, QueueConfig, QueueError, QueuedJob};
pub use regression::spawn_regression_suites_worker;
pub use run_sweeper::{RunSweeperConfig, spawn_run_sweeper};
pub(crate) use sse_proxy::completed_run_status;
pub use sse_proxy::{
    collect_generic_result, collect_generic_result_bytes, proxy_sse_response,
    proxy_sse_response_with_page_actions, update_run_on_completion,
};
pub use state::{
    CreateEventInput, CreateRunInput, EventQuery, ExecutionEventRecord, ExecutionRunRecord,
    ExecutionStateStore, RunMode, RunStatus, RunVariant, StateBackend, StateStoreConfig,
    StateStoreError, UpdateRunInput, create_state_store,
};
pub use wasm_resolve::resolve_wasm_packages;

/// Canonical `ExecutionRun.version` label for a board version tuple —
/// `v{major}-{minor}-{patch}`, the same key the LanceDB run store uses for
/// board versions. Every writer that has the tuple must go through this;
/// `etag:{...}` labels are the one other allowed format.
pub fn format_run_version((major, minor, patch): (u32, u32, u32)) -> String {
    format!("v{major}-{minor}-{patch}")
}

/// Normalize a version label from an external writer (`1.2.3`, `1_2_3`,
/// `v1-2-3`) to the canonical [`format_run_version`] form. Labels that do not
/// parse as a version tuple — `etag:{...}` included — pass through verbatim so
/// readers keep their existing tolerance.
pub fn normalize_run_version_label(label: &str) -> String {
    fn tuple<'a>(mut parts: impl Iterator<Item = &'a str>) -> Option<(u32, u32, u32)> {
        let version = (
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(version)
    }

    let trimmed = label.trim();
    let parsed = match trimmed.strip_prefix('v') {
        Some(rest) => tuple(rest.split('-')),
        None => tuple(trimmed.split(['.', '_'])),
    };
    parsed
        .map(format_run_version)
        .unwrap_or_else(|| label.to_string())
}

#[cfg(test)]
mod version_label_tests {
    use super::{format_run_version, normalize_run_version_label};

    #[test]
    fn run_version_label_is_the_lance_board_version_format() {
        assert_eq!(format_run_version((1, 0, 3)), "v1-0-3");
        assert_eq!(normalize_run_version_label("1.0.3"), "v1-0-3");
        assert_eq!(normalize_run_version_label("1_0_3"), "v1-0-3");
        assert_eq!(normalize_run_version_label("v1-0-3"), "v1-0-3");
    }

    #[test]
    fn non_tuple_labels_pass_through_verbatim() {
        assert_eq!(normalize_run_version_label("etag:abc123"), "etag:abc123");
        assert_eq!(normalize_run_version_label("latest"), "latest");
        assert_eq!(normalize_run_version_label("1.2"), "1.2");
        assert_eq!(normalize_run_version_label("1.2.3.4"), "1.2.3.4");
    }
}
