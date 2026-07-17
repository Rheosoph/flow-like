//! App-connection JWT module for app-to-app authentication
//!
//! When a flow running in one app (the origin) needs to work with another app
//! (the target) it was granted access to, the API mints a short-lived JWT that
//! pins **both** the origin and the target app id. The target-side middleware
//! verifies the signature, expiry, and that the token's `target_app_id` matches
//! the app addressed by the request, so a leaked token cannot be replayed
//! against any other app.

use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use serde::{Deserialize, Serialize};

pub type AppConnectionJwtError = BackendJwtError;

/// Maximum TTL for app-connection tokens in seconds.
pub const MAX_APP_CONNECTION_TTL_SECONDS: i64 = 15 * 60;

/// Maximum length of the `app_chain` claim (calling apps, excluding the
/// target). A chain of 8 means up to 9 apps participate in a single call path.
pub const MAX_APP_CONNECTION_CHAIN: usize = 8;

/// Claims contained in an app-connection JWT
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConnectionClaims {
    /// The user that originally initiated the call chain, passed through
    /// end-to-end so the called app can attribute the call even if the user
    /// is not a member of it. None if the chain was started by an unattended
    /// principal (e.g. a legacy API key without a creator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// The app the call originates from
    pub origin_app_id: String,
    /// The app the token grants access to. Verified against the request path.
    pub target_app_id: String,
    /// The full chain of apps that led to this call, in order; the last
    /// element is `origin_app_id`. For A -> B -> C, the token B mints for C
    /// carries ["A", "B"], keeping the chain transparent for the target.
    #[serde(default)]
    pub app_chain: Vec<String>,
    /// Optional technical user/API key that initiated the call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technical_user_id: Option<String>,
    /// Optional run ID if minted during a flow execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Process-mining correlation (trace root + business keys) propagated from
    /// the calling run so the target inherits the case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<crate::correlation::CorrelationContext>,
    /// Token type - always AppConnection
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

/// Parameters for creating an app-connection JWT
#[derive(Debug, Clone)]
pub struct AppConnectionJwtParams {
    /// The user that originally initiated the call chain, if any
    pub sub: Option<String>,
    pub origin_app_id: String,
    pub target_app_id: String,
    /// Full chain of apps including the origin as last element. Pass an empty
    /// vec to start a new chain with just the origin.
    pub app_chain: Vec<String>,
    pub technical_user_id: Option<String>,
    pub run_id: Option<String>,
    /// Process-mining correlation propagated from the calling run, if any
    pub correlation: Option<crate::correlation::CorrelationContext>,
    /// TTL in seconds, clamped to [60, MAX_APP_CONNECTION_TTL_SECONDS]
    pub ttl_seconds: Option<i64>,
}

/// Check if app-connection JWT signing is available
pub fn is_configured() -> bool {
    backend_jwt::is_configured()
}

fn validate_chain(app_chain: &[String], origin_app_id: &str) -> Result<(), AppConnectionJwtError> {
    if app_chain.last().map(String::as_str) != Some(origin_app_id) {
        return Err(BackendJwtError::EncodingError(
            "App chain must end with the origin app".to_string(),
        ));
    }
    if app_chain.len() > MAX_APP_CONNECTION_CHAIN {
        return Err(BackendJwtError::EncodingError(format!(
            "App connection chain exceeds the maximum depth of {}",
            MAX_APP_CONNECTION_CHAIN
        )));
    }
    Ok(())
}

/// Sign an app-connection JWT with the configured private key
pub fn sign(params: AppConnectionJwtParams) -> Result<String, AppConnectionJwtError> {
    let mut app_chain = params.app_chain;
    if app_chain.is_empty() {
        app_chain.push(params.origin_app_id.clone());
    }
    validate_chain(&app_chain, &params.origin_app_id)?;

    let ttl = params
        .ttl_seconds
        .unwrap_or_else(|| TokenType::AppConnection.default_ttl_seconds())
        .clamp(60, MAX_APP_CONNECTION_TTL_SECONDS);
    let time = make_time_claims(TokenType::AppConnection, Some(ttl));

    let claims = AppConnectionClaims {
        sub: params.sub,
        origin_app_id: params.origin_app_id,
        target_app_id: params.target_app_id,
        app_chain,
        technical_user_id: params.technical_user_id,
        run_id: params.run_id,
        correlation: params.correlation,
        token_type: TokenType::AppConnection,
        iss: issuer().to_string(),
        aud: TokenType::AppConnection.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };

    backend_jwt::sign(&claims)
}

/// Verify and decode an app-connection JWT
pub fn verify(token: &str) -> Result<AppConnectionClaims, AppConnectionJwtError> {
    let mut claims: AppConnectionClaims = backend_jwt::verify(token, TokenType::AppConnection)?;

    if claims.token_type != TokenType::AppConnection {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: TokenType::AppConnection,
            got: claims.token_type,
        });
    }

    if claims.app_chain.is_empty() {
        claims.app_chain = vec![claims.origin_app_id.clone()];
    }
    if claims.app_chain.last() != Some(&claims.origin_app_id)
        || claims.app_chain.len() > MAX_APP_CONNECTION_CHAIN
    {
        return Err(BackendJwtError::DecodingError(
            "Invalid app connection chain".to_string(),
        ));
    }

    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_must_end_with_origin() {
        let chain = vec!["app_a".to_string(), "app_b".to_string()];
        assert!(validate_chain(&chain, "app_b").is_ok());
        assert!(validate_chain(&chain, "app_a").is_err());
        assert!(validate_chain(&chain, "app_c").is_err());
        assert!(validate_chain(&[], "app_a").is_err());
    }

    #[test]
    fn test_chain_depth_cap() {
        let mut chain: Vec<String> = (0..MAX_APP_CONNECTION_CHAIN)
            .map(|i| format!("app_{}", i))
            .collect();
        let origin = chain.last().unwrap().clone();
        assert!(validate_chain(&chain, &origin).is_ok());

        chain.insert(0, "app_extra".to_string());
        assert!(validate_chain(&chain, &origin).is_err());
    }

    #[test]
    fn test_claims_backwards_compat_defaults() {
        // Tokens minted before sub/app_chain existed must still verify:
        // sub defaults to None, app_chain to empty (normalized by verify()).
        let legacy = r#"{
            "origin_app_id": "app_a",
            "target_app_id": "app_b",
            "typ": "app_connection",
            "iss": "flow-like",
            "aud": "flow-like-app-connection",
            "iat": 1, "nbf": 1, "exp": 2, "jti": "x"
        }"#;
        let claims: AppConnectionClaims =
            flow_like_types::json::from_str(legacy).expect("legacy claims must deserialize");
        assert_eq!(claims.sub, None);
        assert!(claims.app_chain.is_empty());
        assert_eq!(claims.technical_user_id, None);
        assert_eq!(claims.run_id, None);
        assert_eq!(claims.correlation, None);
    }

    #[test]
    fn test_app_connection_jwt_roundtrip() {
        if !is_configured() {
            return;
        }

        let correlation = crate::correlation::CorrelationContext {
            trace_id: Some("run_root".to_string()),
            keys: std::collections::HashMap::from([("order_id".to_string(), "1234".to_string())]),
        };
        let params = AppConnectionJwtParams {
            sub: Some("user123".to_string()),
            origin_app_id: "app_origin".to_string(),
            target_app_id: "app_target".to_string(),
            app_chain: vec!["app_first".to_string(), "app_origin".to_string()],
            technical_user_id: None,
            run_id: Some("run456".to_string()),
            correlation: Some(correlation.clone()),
            ttl_seconds: Some(300),
        };

        let token = sign(params.clone()).expect("Failed to sign JWT");
        let claims = verify(&token).expect("Failed to verify JWT");

        assert_eq!(claims.sub, params.sub);
        assert_eq!(claims.origin_app_id, params.origin_app_id);
        assert_eq!(claims.target_app_id, params.target_app_id);
        assert_eq!(claims.app_chain, params.app_chain);
        assert_eq!(claims.run_id, params.run_id);
        assert_eq!(claims.correlation, Some(correlation));
    }
}
