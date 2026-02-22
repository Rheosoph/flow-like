//! Unified Backend JWT module for all API authentication needs
//!
//! This module provides a single keypair for all backend JWT operations:
//! - **Executor tokens**: For execution environments to call back to the API
//! - **User tokens**: For users to poll execution status
//! - **Realtime tokens**: For y-webrtc collaboration
//! - **InteractionResponder tokens**: For users to respond to interactions
//!
//! IMPORTANT: The keypair must be injected at deploy time via environment variables
//! to support horizontal scaling. All API instances must use the same keypair.
//!
//! Environment variables:
//! - `BACKEND_KEY`: Base64-encoded PEM private key (ES256/P-256)
//! - `BACKEND_PUB`: Base64-encoded PEM public key (ES256/P-256)
//! - `BACKEND_KID`: Key identifier (defaults to "backend-es256-v1")

use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use p256::{
    PublicKey as P256PublicKey, elliptic_curve::sec1::ToEncodedPoint, pkcs8::DecodePublicKey,
};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// ============================================================================
// Environment Variables
// ============================================================================

/// Environment variable for the base64-encoded PEM private key
pub const BACKEND_KEY_ENV: &str = "BACKEND_KEY";
/// Environment variable for the base64-encoded PEM public key
pub const BACKEND_PUB_ENV: &str = "BACKEND_PUB";
/// Environment variable for the key identifier
pub const BACKEND_KID_ENV: &str = "BACKEND_KID";

const ISSUER: &str = "flow-like";

// ============================================================================
// Token Types & Audiences
// ============================================================================

/// Token type - determines what the token can be used for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    /// Token for executors to call back to API (progress, events)
    Executor,
    /// Token for compilation workers to call back to API (status)
    Compiler,
    /// Token for users to poll execution status
    User,
    /// Token for realtime collaboration (y-webrtc)
    Realtime,
    /// Token for users to respond to interactions
    InteractionResponder,
}

impl TokenType {
    /// Get the audience string for this token type
    pub fn audience(&self) -> &'static str {
        match self {
            TokenType::Executor => "flow-like-executor",
            TokenType::Compiler => "flow-like-compiler",
            TokenType::User => "flow-like-user",
            TokenType::Realtime => "y-webrtc",
            TokenType::InteractionResponder => "flow-like-interaction-responder",
        }
    }

    /// Get the default TTL in seconds for this token type
    pub fn default_ttl_seconds(&self) -> i64 {
        match self {
            TokenType::Executor => 24 * 60 * 60,       // 24 hours
            TokenType::Compiler => 24 * 60 * 60,       // 24 hours
            TokenType::User => 60 * 60,                // 1 hour
            TokenType::Realtime => 3 * 60 * 60,        // 3 hours
            TokenType::InteractionResponder => 5 * 60, // 5 minutes
        }
    }
}

// ============================================================================
// Static Key Storage
// ============================================================================

/// Lazily loaded private key for signing JWTs
static PRIVATE_KEY_PEM: LazyLock<Option<Vec<u8>>> = LazyLock::new(|| {
    std::env::var(BACKEND_KEY_ENV)
        .ok()
        .and_then(|b64| STANDARD.decode(&b64).ok())
});

/// Lazily loaded public key for verifying JWTs
static PUBLIC_KEY_PEM: LazyLock<Option<Vec<u8>>> = LazyLock::new(|| {
    std::env::var(BACKEND_PUB_ENV)
        .ok()
        .and_then(|b64| STANDARD.decode(&b64).ok())
});

/// Key identifier for JWKS
static KID: LazyLock<String> = LazyLock::new(|| {
    std::env::var(BACKEND_KID_ENV).unwrap_or_else(|_| "backend-es256-v1".to_string())
});

// ============================================================================
// Error Type
// ============================================================================

/// Error type for backend JWT operations
#[derive(Debug)]
pub enum BackendJwtError {
    MissingPrivateKey,
    MissingPublicKey,
    EncodingError(String),
    DecodingError(String),
    InvalidPublicKey(String),
    TokenTypeMismatch { expected: TokenType, got: TokenType },
}

impl std::fmt::Display for BackendJwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendJwtError::MissingPrivateKey => write!(
                f,
                "Backend signing key not configured (missing {} env var)",
                BACKEND_KEY_ENV
            ),
            BackendJwtError::MissingPublicKey => write!(
                f,
                "Backend public key not configured (missing {} env var)",
                BACKEND_PUB_ENV
            ),
            BackendJwtError::EncodingError(msg) => write!(f, "Failed to encode JWT: {}", msg),
            BackendJwtError::DecodingError(msg) => write!(f, "Failed to decode JWT: {}", msg),
            BackendJwtError::InvalidPublicKey(msg) => write!(f, "Invalid public key: {}", msg),
            BackendJwtError::TokenTypeMismatch { expected, got } => {
                write!(
                    f,
                    "Token type mismatch: expected {:?}, got {:?}",
                    expected, got
                )
            }
        }
    }
}

impl std::error::Error for BackendJwtError {}

// ============================================================================
// Configuration Check
// ============================================================================

/// Check if backend JWT signing is available
pub fn is_configured() -> bool {
    PRIVATE_KEY_PEM.is_some() && PUBLIC_KEY_PEM.is_some()
}

/// Get the key identifier
pub fn get_kid() -> String {
    KID.clone()
}

// ============================================================================
// Signing
// ============================================================================

/// Sign a JWT with custom claims
///
/// The claims must include a `typ` field with `TokenType` and standard JWT fields.
pub fn sign<T: Serialize>(claims: &T) -> Result<String, BackendJwtError> {
    let private_key = PRIVATE_KEY_PEM
        .as_ref()
        .ok_or(BackendJwtError::MissingPrivateKey)?;

    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(KID.clone());

    let encoding_key = EncodingKey::from_ec_pem(private_key)
        .map_err(|e| BackendJwtError::EncodingError(e.to_string()))?;

    encode(&header, claims, &encoding_key)
        .map_err(|e| BackendJwtError::EncodingError(e.to_string()))
}

// ============================================================================
// Verification
// ============================================================================

/// Verify and decode a JWT with expected token type
///
/// Validates issuer, audience (based on token type), and expiration.
pub fn verify<T: for<'de> Deserialize<'de>>(
    token: &str,
    expected_type: TokenType,
) -> Result<T, BackendJwtError> {
    let public_key = PUBLIC_KEY_PEM
        .as_ref()
        .ok_or(BackendJwtError::MissingPublicKey)?;

    let decoding_key = DecodingKey::from_ec_pem(public_key)
        .map_err(|e| BackendJwtError::DecodingError(e.to_string()))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[expected_type.audience()]);

    let token_data = decode::<T>(token, &decoding_key, &validation)
        .map_err(|e| BackendJwtError::DecodingError(e.to_string()))?;

    Ok(token_data.claims)
}

/// Verify a JWT without checking audience (for introspection)
pub fn verify_any<T: for<'de> Deserialize<'de>>(token: &str) -> Result<T, BackendJwtError> {
    let public_key = PUBLIC_KEY_PEM
        .as_ref()
        .ok_or(BackendJwtError::MissingPublicKey)?;

    let decoding_key = DecodingKey::from_ec_pem(public_key)
        .map_err(|e| BackendJwtError::DecodingError(e.to_string()))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_issuer(&[ISSUER]);
    validation.validate_aud = false;

    let token_data = decode::<T>(token, &decoding_key, &validation)
        .map_err(|e| BackendJwtError::DecodingError(e.to_string()))?;

    Ok(token_data.claims)
}

// ============================================================================
// JWKS (JSON Web Key Set)
// ============================================================================

/// JWK representation for JWKS endpoint
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Jwk {
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub y: String,
    pub alg: String,
    pub kid: String,
    #[serde(rename = "use")]
    pub r#use: String,
}

/// JWKS (JSON Web Key Set) for public key distribution
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

/// Get the JWKS containing the public key for JWT verification
pub fn get_jwks() -> Result<Jwks, BackendJwtError> {
    let public_key_pem = PUBLIC_KEY_PEM
        .as_ref()
        .ok_or(BackendJwtError::MissingPublicKey)?;

    let pem = String::from_utf8_lossy(public_key_pem);
    let pubkey: P256PublicKey = P256PublicKey::from_public_key_pem(&pem)
        .map_err(|e| BackendJwtError::InvalidPublicKey(e.to_string()))?;

    let encoded = pubkey.to_encoded_point(false); // uncompressed
    let x = encoded
        .x()
        .ok_or_else(|| BackendJwtError::InvalidPublicKey("Missing X coord".to_string()))?;
    let y = encoded
        .y()
        .ok_or_else(|| BackendJwtError::InvalidPublicKey("Missing Y coord".to_string()))?;

    let jwk = Jwk {
        kty: "EC".to_string(),
        crv: "P-256".to_string(),
        x: URL_SAFE_NO_PAD.encode(x),
        y: URL_SAFE_NO_PAD.encode(y),
        alg: "ES256".to_string(),
        kid: KID.clone(),
        r#use: "sig".to_string(),
    };

    Ok(Jwks { keys: vec![jwk] })
}

// ============================================================================
// Helpers for building claims
// ============================================================================

/// Standard JWT time claims
pub struct TimeClaims {
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
}

/// Generate standard time claims for a token
pub fn make_time_claims(token_type: TokenType, ttl_override: Option<i64>) -> TimeClaims {
    let iat = chrono::Utc::now().timestamp();
    let ttl = ttl_override.unwrap_or_else(|| token_type.default_ttl_seconds());
    TimeClaims {
        iat,
        nbf: iat - 30, // 30 second clock skew allowance
        exp: iat + ttl,
    }
}

/// Get the issuer string
pub fn issuer() -> &'static str {
    ISSUER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct TestClaims {
        sub: String,
        #[serde(rename = "typ")]
        token_type: TokenType,
        iss: String,
        aud: String,
        iat: i64,
        nbf: i64,
        exp: i64,
    }

    #[test]
    fn test_jwt_roundtrip() {
        if !is_configured() {
            return;
        }

        let time = make_time_claims(TokenType::Executor, None);
        let claims = TestClaims {
            sub: "test-user".to_string(),
            token_type: TokenType::Executor,
            iss: ISSUER.to_string(),
            aud: TokenType::Executor.audience().to_string(),
            iat: time.iat,
            nbf: time.nbf,
            exp: time.exp,
        };

        let token = sign(&claims).expect("Failed to sign");
        let decoded: TestClaims = verify(&token, TokenType::Executor).expect("Failed to verify");

        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.token_type, TokenType::Executor);
    }
}
