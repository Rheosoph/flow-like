//! Execution JWT module for runtime authentication
//!
//! This module provides execution-specific JWT claims and helpers,
//! using the unified backend JWT module for signing and verification.
//!
//! Two types of execution JWTs are supported:
//! - **Executor JWTs**: Given to execution environments (K8s, Docker, Lambda) to call back to the API
//! - **User JWTs**: Returned to users for long polling execution status

use crate::backend_jwt::{self, BackendJwtError, Jwk, Jwks, TokenType, issuer, make_time_claims};
use serde::{Deserialize, Serialize};

pub type ExecutionJwk = Jwk;
pub type ExecutionJwks = Jwks;

/// Execution JWT error type (wraps BackendJwtError)
pub type ExecutionJwtError = BackendJwtError;

/// Signed context needed to sanitize actions created during a Page run.
/// Executors may report output with this token, but cannot alter the Page
/// identity or authority revision used by the API delivery boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageExecutionJwtContext {
    pub page_id: String,
    pub manifest_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_version: Option<(u32, u32, u32)>,
    /// Exact object identity when the Event selector is Latest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_etag: Option<String>,
    /// The one entry selected by this invocation. The executor checks exact
    /// equality so a queued payload cannot substitute another allowed entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_node_id: Option<String>,
    /// Signature of the exact prerun manifest that supplies the dynamic
    /// action allow-list at the callback boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_authority_revision: Option<String>,
    /// Revision of the exact WASM bundle carried by this dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_authority_revision: Option<String>,
    /// Signed allow-list used by the callback boundary to seal dynamic output
    /// by issuers that predate compact prerun-manifest authority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_entry_node_ids: Vec<String>,
}

/// Claims contained in an execution JWT
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionClaims {
    /// Subject - the user ID who initiated the execution
    pub sub: String,
    /// Optional technical user/API key that initiated the execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technical_user_id: Option<String>,
    /// The run ID (unique per execution)
    pub run_id: String,
    /// The application ID
    pub app_id: String,
    /// The board ID being executed
    pub board_id: String,
    /// Optional event ID if triggered by an event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// If the run was triggered through an app connection: the chain of apps
    /// that led to it (last element = the app that made the call). Passed
    /// through so tokens minted during this run keep the chain transparent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_chain: Option<Vec<String>>,
    /// Process-mining correlation (trace root + business keys), propagated so
    /// tokens minted during this run pass it to downstream apps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<crate::correlation::CorrelationContext>,
    /// Present only for a run resolved through a governed Page trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_execution: Option<PageExecutionJwtContext>,
    /// Signed shadow/replay isolation flag. The executor derives the effective
    /// flag from this claim and rejects a payload whose `shadow` byte differs;
    /// `None` (old tokens) means a normal run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    /// Callback URL for progress/event reporting
    pub callback_url: String,
    /// Token type - executor or user
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    /// Issuer
    pub iss: String,
    /// Audience
    pub aud: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Not before (Unix timestamp)
    pub nbf: i64,
    /// Expiration (Unix timestamp)
    pub exp: i64,
    /// JWT ID (unique token identifier)
    pub jti: String,
}

/// Parameters for creating an execution JWT
#[derive(Debug, Clone)]
pub struct ExecutionJwtParams {
    pub user_id: String,
    pub technical_user_id: Option<String>,
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
    pub event_id: Option<String>,
    /// Chain of apps that led to this run via app connections, if any
    pub app_chain: Option<Vec<String>>,
    /// Process-mining correlation to propagate downstream, if any
    pub correlation: Option<crate::correlation::CorrelationContext>,
    pub callback_url: String,
    /// Token type - executor or user
    pub token_type: TokenType,
    /// TTL in seconds (defaults based on token type)
    pub ttl_seconds: Option<i64>,
    /// Shadow/replay isolation, signed into the claims. `None` for normal runs.
    pub shadow: Option<bool>,
}

/// Check if execution JWT signing is available
pub fn is_configured() -> bool {
    backend_jwt::is_configured()
}

/// Sign an execution JWT with the configured private key
pub fn sign(params: ExecutionJwtParams) -> Result<String, ExecutionJwtError> {
    sign_inner(params, None)
}

/// Sign an executor token for a governed Page run. The context is later used
/// to seal dynamic A2UI actions before they are delivered to a user.
pub fn sign_with_page_context(
    params: ExecutionJwtParams,
    page_execution: PageExecutionJwtContext,
) -> Result<String, ExecutionJwtError> {
    sign_inner(params, Some(page_execution))
}

fn sign_inner(
    params: ExecutionJwtParams,
    page_execution: Option<PageExecutionJwtContext>,
) -> Result<String, ExecutionJwtError> {
    let time = make_time_claims(params.token_type, params.ttl_seconds);

    let claims = ExecutionClaims {
        sub: params.user_id,
        technical_user_id: params.technical_user_id,
        run_id: params.run_id,
        app_id: params.app_id,
        board_id: params.board_id,
        event_id: params.event_id,
        app_chain: params.app_chain,
        correlation: params.correlation,
        page_execution,
        shadow: params.shadow,
        callback_url: params.callback_url,
        token_type: params.token_type,
        iss: issuer().to_string(),
        aud: params.token_type.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };

    backend_jwt::sign(&claims)
}

/// Verify and decode an execution JWT for executors
pub fn verify(token: &str) -> Result<ExecutionClaims, ExecutionJwtError> {
    verify_with_type(token, TokenType::Executor)
}

/// Verify and decode an execution JWT for users (long polling)
pub fn verify_user(token: &str) -> Result<ExecutionClaims, ExecutionJwtError> {
    verify_with_type(token, TokenType::User)
}

/// Verify and decode an execution JWT with specific token type
pub fn verify_with_type(
    token: &str,
    expected_type: TokenType,
) -> Result<ExecutionClaims, ExecutionJwtError> {
    let claims: ExecutionClaims = backend_jwt::verify(token, expected_type)?;

    // Double-check token type claim matches
    if claims.token_type != expected_type {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: expected_type,
            got: claims.token_type,
        });
    }

    Ok(claims)
}

/// Get the JWKS (delegates to backend_jwt)
pub fn get_jwks() -> Result<Jwks, ExecutionJwtError> {
    backend_jwt::get_jwks()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_roundtrip() {
        if !is_configured() {
            return;
        }

        let params = ExecutionJwtParams {
            user_id: "user123".to_string(),
            technical_user_id: None,
            run_id: "run456".to_string(),
            app_id: "app789".to_string(),
            board_id: "board012".to_string(),
            event_id: Some("event345".to_string()),
            app_chain: None,
            correlation: None,
            callback_url: "http://localhost:8080".to_string(),
            token_type: TokenType::Executor,
            ttl_seconds: Some(3600),
            shadow: None,
        };

        let token = sign(params.clone()).expect("Failed to sign JWT");
        let claims = verify(&token).expect("Failed to verify JWT");

        assert_eq!(claims.sub, params.user_id);
        assert_eq!(claims.run_id, params.run_id);
        assert_eq!(claims.app_id, params.app_id);
        assert_eq!(claims.board_id, params.board_id);
        assert_eq!(claims.event_id, params.event_id);
        assert_eq!(claims.callback_url, params.callback_url);
        assert!(claims.page_execution.is_none());
        assert!(claims.shadow.is_none());
    }

    #[test]
    fn shadow_flag_roundtrips_only_when_requested() {
        if !is_configured() {
            return;
        }

        let mut params = ExecutionJwtParams {
            user_id: "user123".to_string(),
            technical_user_id: None,
            run_id: "run456".to_string(),
            app_id: "app789".to_string(),
            board_id: "board012".to_string(),
            event_id: None,
            app_chain: None,
            correlation: None,
            callback_url: "http://localhost:8080".to_string(),
            token_type: TokenType::Executor,
            ttl_seconds: Some(3600),
            shadow: Some(true),
        };

        let token = sign(params.clone()).expect("Failed to sign JWT");
        let claims = verify(&token).expect("Failed to verify JWT");
        assert_eq!(claims.shadow, Some(true));

        params.shadow = None;
        let token = sign(params).expect("Failed to sign JWT");
        let claims = verify(&token).expect("Failed to verify JWT");
        assert!(claims.shadow.is_none());
    }

    #[test]
    fn page_execution_context_roundtrips_only_when_requested() {
        if !is_configured() {
            return;
        }

        let params = ExecutionJwtParams {
            user_id: "user123".to_string(),
            technical_user_id: None,
            run_id: "run456".to_string(),
            app_id: "app789".to_string(),
            board_id: "board012".to_string(),
            event_id: Some("event345".to_string()),
            app_chain: None,
            correlation: None,
            callback_url: "http://localhost:8080".to_string(),
            token_type: TokenType::Executor,
            ttl_seconds: Some(3600),
            shadow: None,
        };
        let page_execution = PageExecutionJwtContext {
            page_id: "page-1".to_string(),
            manifest_revision: "revision-1".to_string(),
            board_version: Some((1, 2, 3)),
            board_etag: None,
            target_node_id: Some("entry-1".to_string()),
            entry_authority_revision: Some("authority-1".to_string()),
            wasm_authority_revision: Some("wasm-1".to_string()),
            allowed_entry_node_ids: vec!["entry-1".to_string()],
        };

        let token = sign_with_page_context(params, page_execution.clone()).unwrap();
        let claims = verify(&token).unwrap();
        assert_eq!(claims.page_execution, Some(page_execution));
    }
}
