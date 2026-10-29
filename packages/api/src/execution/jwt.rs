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
    /// Authenticated payer of an attended stream; absent for every background invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer_sub: Option<String>,
    /// The run serves a hosted frontend visitor: `sub` is the visitor's
    /// identity, and the publication's authority is read from the event's sink
    /// when a callback arrives.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hosted_frontend: bool,
    /// User whose private custom Bits this run may call through the API: the
    /// person the run acts as. A subject an app connection passed through is
    /// one only when it names the person who started the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_bits_owner: Option<String>,
    /// No Flow-Like person triggered the run (a cron, webhook or bot fire, inbound mail, a
    /// public call without a login, a scheduled replay): `sub` is the user the event runs as,
    /// or a placeholder. App connection tokens the run mints pass no person on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unattended: bool,
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
    /// Digest of the complete dispatch envelope, added after transport preparation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_hash: Option<String>,
    /// Workflow duration admitted before dispatch, excluding platform setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_limit_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_receipt_url: Option<String>,
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
    sign_inner(params, None, false)
}

/// Sign the executor token of a run no Flow-Like person triggered, see
/// [`ExecutionClaims::unattended`].
pub fn sign_unattended(params: ExecutionJwtParams) -> Result<String, ExecutionJwtError> {
    sign_inner(params, None, true)
}

/// Sign an executor token for a governed Page run. The context is later used
/// to seal dynamic A2UI actions before they are delivered to a user.
pub fn sign_with_page_context(
    params: ExecutionJwtParams,
    page_execution: PageExecutionJwtContext,
) -> Result<String, ExecutionJwtError> {
    sign_inner(params, Some(page_execution), false)
}

fn sign_inner(
    params: ExecutionJwtParams,
    page_execution: Option<PageExecutionJwtContext>,
    unattended: bool,
) -> Result<String, ExecutionJwtError> {
    if params.token_type == TokenType::Executor
        && params
            .ttl_seconds
            .is_some_and(|ttl| !(1..=86_400).contains(&ttl))
    {
        return Err(BackendJwtError::EncodingError(
            "executor token lifetime must be between 1 second and 24 hours".into(),
        ));
    }
    if unattended && params.token_type != TokenType::Executor {
        return Err(BackendJwtError::EncodingError(
            "only an executor token can mark its run unattended".into(),
        ));
    }
    let time = make_time_claims(params.token_type, params.ttl_seconds);

    let claims = ExecutionClaims {
        sub: params.user_id,
        payer_sub: None,
        hosted_frontend: false,
        custom_bits_owner: None,
        unattended,
        technical_user_id: params.technical_user_id,
        run_id: params.run_id,
        app_id: params.app_id,
        board_id: params.board_id,
        event_id: params.event_id,
        app_chain: params.app_chain,
        correlation: params.correlation,
        page_execution,
        shadow: params.shadow,
        dispatch_hash: None,
        runtime_limit_ms: None,
        quota_receipt_url: None,
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

pub(crate) fn bind_attended_payer(token: &str, payer: &str) -> Result<String, ExecutionJwtError> {
    let mut claims = verify(token)?;
    if !attended_payer_allowed(&claims, payer) {
        return Err(BackendJwtError::EncodingError(
            "Payment payer must be the attended user".into(),
        ));
    }
    claims.payer_sub = Some(payer.to_owned());
    backend_jwt::sign(&claims)
}

pub(crate) fn bind_hosted_frontend(token: &str) -> Result<String, ExecutionJwtError> {
    let mut claims = verify(token)?;
    if !hosted_frontend_allowed(&claims) {
        return Err(BackendJwtError::EncodingError(
            "Hosted frontend authority requires an owner-funded Event executor without an API key or app chain"
                .into(),
        ));
    }
    claims.hosted_frontend = true;
    backend_jwt::sign(&claims)
}

pub(crate) fn bind_custom_bits_owner(
    token: &str,
    owner: &str,
) -> Result<String, ExecutionJwtError> {
    let mut claims = verify(token)?;
    claims.custom_bits_owner = Some(owner.to_owned());
    backend_jwt::sign(&claims)
}

fn is_unchained(claims: &ExecutionClaims) -> bool {
    claims
        .app_chain
        .as_ref()
        .is_none_or(|chain| chain.is_empty())
}

fn hosted_frontend_allowed(claims: &ExecutionClaims) -> bool {
    claims.token_type == TokenType::Executor
        && claims.event_id.is_some()
        && claims.payer_sub.is_none()
        && !claims.shadow.unwrap_or(false)
        && !claims.unattended
        && claims.technical_user_id.is_none()
        && is_unchained(claims)
}

fn attended_payer_allowed(claims: &ExecutionClaims, payer: &str) -> bool {
    claims.sub == payer
        && !claims.hosted_frontend
        && !claims.unattended
        && claims.technical_user_id.is_none()
        && is_unchained(claims)
        && !claims.shadow.unwrap_or(false)
        && claims.token_type == TokenType::Executor
}

pub(super) fn bind_dispatch(token: &str, hash: String) -> Result<String, ExecutionJwtError> {
    let mut claims = verify(token)?;
    if std::env::var("EXECUTION_ISOLATION_MODE").as_deref() == Ok("per_run")
        && (claims.exp.saturating_sub(claims.iat) > 86_400
            || claims.exp > chrono::Utc::now().timestamp().saturating_add(86_400))
    {
        return Err(BackendJwtError::EncodingError(
            "per-run dispatch cannot outlive cancellation retention (24 hours)".into(),
        ));
    }
    claims.dispatch_hash = Some(hash);
    backend_jwt::sign(&claims)
}

pub(super) fn bind_runtime_limit(
    token: &str,
    runtime_limit_ms: u64,
    receipt_url: Option<String>,
) -> Result<String, ExecutionJwtError> {
    let mut claims = verify(token)?;
    claims.runtime_limit_ms = Some(runtime_limit_ms);
    claims.quota_receipt_url = receipt_url;
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

        assert!(claims.payer_sub.is_none());
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
    fn legacy_claims_do_not_infer_a_payer() {
        let value = serde_json::json!({"sub":"user","run_id":"run","app_id":"app","board_id":"board","callback_url":"https://api.example","typ":"executor","iss":"flow-like","aud":"flow-like-executor","iat":1,"nbf":1,"exp":100,"jti":"token"});
        let claims: ExecutionClaims = serde_json::from_value(value).unwrap();
        assert!(claims.payer_sub.is_none());
        assert!(!claims.hosted_frontend);
        assert!(claims.custom_bits_owner.is_none());
        assert!(!claims.unattended);
        let value = serde_json::to_value(claims).unwrap();
        assert!(value.get("payer_sub").is_none());
        assert!(value.get("custom_bits_owner").is_none());
        assert!(value.get("unattended").is_none());
    }

    fn executor_params() -> ExecutionJwtParams {
        ExecutionJwtParams {
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
        }
    }

    fn page_context(entry_node_ids: Vec<String>) -> PageExecutionJwtContext {
        PageExecutionJwtContext {
            page_id: flow_like_types::create_id(),
            manifest_revision: "m".repeat(64),
            board_version: Some((u32::MAX, u32::MAX, u32::MAX)),
            board_etag: Some("e".repeat(64)),
            target_node_id: entry_node_ids.first().cloned(),
            entry_authority_revision: Some("a".repeat(64)),
            wasm_authority_revision: Some("w".repeat(64)),
            allowed_entry_node_ids: entry_node_ids,
        }
    }

    fn signed_payload(token: &str) -> serde_json::Value {
        use base64::Engine;
        let payload = token
            .split('.')
            .nth(1)
            .expect("a JWT has a payload segment");
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("the payload is base64url");
        serde_json::from_slice(&bytes).expect("the payload is JSON")
    }

    #[test]
    fn custom_bits_owner_round_trips_and_is_absent_by_default() {
        crate::backend_jwt::init_for_tests();
        let token = sign(executor_params()).unwrap();
        assert!(verify(&token).unwrap().custom_bits_owner.is_none());
        assert!(signed_payload(&token).get("custom_bits_owner").is_none());

        let bound = bind_custom_bits_owner(&token, "owner-1").unwrap();
        assert_eq!(
            verify(&bound).unwrap().custom_bits_owner.as_deref(),
            Some("owner-1")
        );
        assert_eq!(signed_payload(&bound)["custom_bits_owner"], "owner-1");
    }

    /// Only a run no person triggered is marked unattended, and the mark survives every later
    /// binding of the token.
    #[test]
    fn unattended_runs_are_marked_and_keep_the_mark_through_every_binding() {
        crate::backend_jwt::init_for_tests();
        let attended = sign(executor_params()).unwrap();
        assert!(!verify(&attended).unwrap().unattended);
        assert!(signed_payload(&attended).get("unattended").is_none());

        let token = sign_unattended(executor_params()).unwrap();
        assert_eq!(signed_payload(&token)["unattended"], true);
        let token = bind_custom_bits_owner(&token, "run-as-1").unwrap();
        let token = bind_runtime_limit(&token, 30_000, None).unwrap();
        let token = bind_dispatch(&token, "d".repeat(64)).unwrap();
        let claims = verify(&token).unwrap();
        assert!(claims.unattended);
        assert_eq!(claims.sub, "user123");
        assert_eq!(claims.custom_bits_owner.as_deref(), Some("run-as-1"));

        let mut user_params = executor_params();
        user_params.token_type = TokenType::User;
        assert!(sign_unattended(user_params).is_err());
    }

    /// A hosted visit and an attended stream are runs a person started, so an unattended
    /// token can become neither.
    #[test]
    fn unattended_runs_never_serve_a_visitor_or_an_attended_payer() {
        crate::backend_jwt::init_for_tests();
        let token = sign_unattended(executor_params()).unwrap();
        assert!(bind_hosted_frontend(&token).is_err());
        assert!(bind_attended_payer(&token, "user123").is_err());
        let mut claims = verify(&token).unwrap();
        assert!(!attended_payer_allowed(&claims, "user123"));
        claims.unattended = false;
        assert!(attended_payer_allowed(&claims, "user123"));
        assert!(hosted_frontend_allowed(&claims));
    }

    #[test]
    fn bind_custom_bits_owner_refuses_user_tokens_and_keeps_other_claims() {
        crate::backend_jwt::init_for_tests();
        let mut user_params = executor_params();
        user_params.token_type = TokenType::User;
        let user_token = sign(user_params).unwrap();
        assert!(bind_custom_bits_owner(&user_token, "owner-1").is_err());

        let page = page_context(vec!["entry-1".to_string()]);
        let token = sign_with_page_context(executor_params(), page.clone()).unwrap();
        let token = bind_hosted_frontend(&token).unwrap();
        let receipt_url = "https://receipts.example/run456".to_string();
        let token = bind_runtime_limit(&token, 30_000, Some(receipt_url.clone())).unwrap();
        let claims = verify(&bind_custom_bits_owner(&token, "owner-1").unwrap()).unwrap();

        assert_eq!(claims.custom_bits_owner.as_deref(), Some("owner-1"));
        assert_eq!(claims.page_execution, Some(page));
        assert_eq!(claims.runtime_limit_ms, Some(30_000));
        assert_eq!(claims.quota_receipt_url, Some(receipt_url));
        assert!(claims.hosted_frontend);
        assert_eq!(claims.sub, "user123");
    }

    #[test]
    fn hosted_frontend_binding_requires_an_unchained_human_event_executor() {
        crate::backend_jwt::init_for_tests();
        let hosted = bind_hosted_frontend(&sign(executor_params()).unwrap()).unwrap();
        assert!(verify(&hosted).unwrap().hosted_frontend);

        let refused = |change: fn(&mut ExecutionJwtParams)| {
            let mut params = executor_params();
            change(&mut params);
            bind_hosted_frontend(&sign(params).unwrap()).is_err()
        };
        assert!(refused(
            |params| params.technical_user_id = Some("key-1".into())
        ));
        assert!(refused(
            |params| params.app_chain = Some(vec!["calling-app".into()])
        ));
        assert!(refused(|params| params.shadow = Some(true)));
        assert!(refused(|params| params.event_id = None));
        assert!(refused(|params| params.token_type = TokenType::User));

        let funded = bind_attended_payer(&sign(executor_params()).unwrap(), "user123").unwrap();
        assert!(bind_hosted_frontend(&funded).is_err());
    }

    #[test]
    fn executor_token_stays_under_header_limits() {
        crate::backend_jwt::init_for_tests();
        let entry_node_ids = (0..32).map(|_| flow_like_types::create_id()).collect();
        let token =
            sign_with_page_context(executor_params(), page_context(entry_node_ids)).unwrap();
        let receipt_prefix = "https://receipts.example/";
        let receipt_url = format!(
            "{receipt_prefix}{}",
            "x".repeat(2048 - receipt_prefix.len())
        );
        let token = bind_runtime_limit(&token, u64::MAX, Some(receipt_url)).unwrap();
        let token = bind_custom_bits_owner(&token, &"o".repeat(64)).unwrap();
        let token = bind_dispatch(&token, "d".repeat(64)).unwrap();

        assert!(
            token.len() < 8 * 1024,
            "the executor JWT is {} bytes",
            token.len()
        );
    }

    #[test]
    fn payer_authority_cannot_follow_a_technical_shadow_or_chained_subject() {
        let value = serde_json::json!({"sub":"user","run_id":"run","app_id":"app","board_id":"board","callback_url":"https://api.example","typ":"executor","iss":"flow-like","aud":"flow-like-executor","iat":1,"nbf":1,"exp":100,"jti":"token"});
        let claims: ExecutionClaims = serde_json::from_value(value).unwrap();
        assert!(attended_payer_allowed(&claims, "user"));
        assert!(!attended_payer_allowed(&claims, "someone-else"));
        let mut changed = claims.clone();
        changed.hosted_frontend = true;
        assert!(!attended_payer_allowed(&changed, "user"));
        let mut changed = claims.clone();
        changed.technical_user_id = Some("technical".into());
        assert!(!attended_payer_allowed(&changed, "user"));
        let mut changed = claims.clone();
        changed.app_chain = Some(vec!["calling-app".into()]);
        assert!(!attended_payer_allowed(&changed, "user"));
        let mut changed = claims.clone();
        changed.shadow = Some(true);
        assert!(!attended_payer_allowed(&changed, "user"));
        let mut changed = claims;
        changed.token_type = TokenType::User;
        assert!(!attended_payer_allowed(&changed, "user"));
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
