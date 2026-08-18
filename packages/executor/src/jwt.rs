use base64::{engine::general_purpose::STANDARD, Engine};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::error::ExecutorError;

/// Environment variable names for JWT configuration
/// BACKEND_PUB is the unified key used by the API for all JWT types
pub const BACKEND_PUB_ENV: &str = "BACKEND_PUB";
pub const API_URL_ENV: &str = "API_URL";
const CALLBACK_BASE_URL_ENV: &str = "CALLBACK_BASE_URL";

/// Cached public key bytes (fetched from API or env var)
static PUBLIC_KEY_CACHE: OnceCell<Vec<u8>> = OnceCell::const_new();

/// JWKS response from API
#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    kty: String,
    crv: String,
    x: String,
    y: String,
    alg: String,
    kid: String,
}

/// Claims embedded in the executor JWT (must match ExecutionClaims from API)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorClaims {
    /// Subject (user ID)
    pub sub: String,
    /// Run ID for this execution
    pub run_id: String,
    /// Application ID
    pub app_id: String,
    /// Board ID being executed
    pub board_id: String,
    /// Optional event ID if triggered by an event
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Callback URL for progress reporting
    pub callback_url: String,
    /// Token type
    #[serde(rename = "typ")]
    pub token_type: String,
    /// Issuer
    pub iss: String,
    /// Audience
    pub aud: String,
    /// Issued at timestamp
    pub iat: i64,
    /// Not before timestamp
    pub nbf: i64,
    /// Expiration timestamp
    pub exp: i64,
    /// JWT ID
    pub jti: String,
}

/// Get the API base URL for fetching JWKS
fn get_api_url() -> Result<String, ExecutorError> {
    // Try API_URL first, then CALLBACK_BASE_URL
    std::env::var(API_URL_ENV)
        .or_else(|_| std::env::var(CALLBACK_BASE_URL_ENV))
        .map_err(|_| {
            ExecutorError::Config(format!(
                "Neither {} nor {} is set. Cannot verify JWT.",
                BACKEND_PUB_ENV, API_URL_ENV
            ))
        })
}

/// Fetch public key from API's JWKS endpoint
async fn fetch_public_key_from_api() -> Result<Vec<u8>, ExecutorError> {
    let api_url = get_api_url()?;
    let jwks_url = format!(
        "{}/execution/.well-known/jwks.json",
        api_url.trim_end_matches('/')
    );

    tracing::info!(url = %jwks_url, "Fetching execution JWKS from API");

    // The edge WAF blocks UA-less requests (CRS `NoUserAgent_HEADER`), and
    // this fetch goes through the same distribution as the callbacks.
    let client = reqwest::Client::builder()
        .user_agent(concat!("flow-like-executor/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| ExecutorError::Jwt(format!("Failed to build JWKS client: {}", e)))?;
    let response = client
        .get(&jwks_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| ExecutorError::Jwt(format!("Failed to fetch JWKS: {}", e)))?;

    if !response.status().is_success() {
        return Err(ExecutorError::Jwt(format!(
            "JWKS endpoint returned status {}",
            response.status()
        )));
    }

    let jwks: Jwks = response
        .json()
        .await
        .map_err(|e| ExecutorError::Jwt(format!("Failed to parse JWKS: {}", e)))?;

    let jwk = jwks
        .keys
        .first()
        .ok_or_else(|| ExecutorError::Jwt("JWKS contains no keys".to_string()))?;

    if jwk.kty != "EC" || jwk.crv != "P-256" {
        return Err(ExecutorError::Jwt(format!(
            "Unsupported key type: {} {}",
            jwk.kty, jwk.crv
        )));
    }

    // Decode x and y coordinates from base64url
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let x_bytes = URL_SAFE_NO_PAD
        .decode(&jwk.x)
        .map_err(|e| ExecutorError::Jwt(format!("Failed to decode x coordinate: {}", e)))?;
    let y_bytes = URL_SAFE_NO_PAD
        .decode(&jwk.y)
        .map_err(|e| ExecutorError::Jwt(format!("Failed to decode y coordinate: {}", e)))?;

    // Construct uncompressed SEC1 point: 0x04 || x || y
    let mut point = Vec::with_capacity(1 + x_bytes.len() + y_bytes.len());
    point.push(0x04);
    point.extend_from_slice(&x_bytes);
    point.extend_from_slice(&y_bytes);

    // Convert to PEM format using p256 crate
    use p256::elliptic_curve::sec1::FromEncodedPoint;
    use p256::EncodedPoint;
    use p256::PublicKey;

    let encoded_point = EncodedPoint::from_bytes(&point)
        .map_err(|e| ExecutorError::Jwt(format!("Invalid EC point: {}", e)))?;

    let public_key = PublicKey::from_encoded_point(&encoded_point);
    if public_key.is_none().into() {
        return Err(ExecutorError::Jwt("Invalid EC public key".to_string()));
    }
    let public_key = public_key.unwrap();

    use p256::pkcs8::EncodePublicKey;
    let pem = public_key
        .to_public_key_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| ExecutorError::Jwt(format!("Failed to encode public key: {}", e)))?;

    Ok(pem.into_bytes())
}

/// Get public key bytes - from env var or fetched from API
async fn get_public_key() -> Result<&'static Vec<u8>, ExecutorError> {
    PUBLIC_KEY_CACHE
        .get_or_try_init(|| async {
            // First try env var
            if let Ok(b64) = std::env::var(BACKEND_PUB_ENV) {
                tracing::info!("Using {} from environment", BACKEND_PUB_ENV);
                return STANDARD.decode(&b64).map_err(|e| {
                    ExecutorError::Jwt(format!("Failed to decode {}: {}", BACKEND_PUB_ENV, e))
                });
            }

            // Fall back to fetching from API
            fetch_public_key_from_api().await
        })
        .await
}

/// Verify and decode the executor JWT (async version - fetches key from API if needed)
pub async fn verify_jwt_async(token: &str) -> Result<ExecutorClaims, ExecutorError> {
    let key_bytes = get_public_key().await?;

    let decoding_key = DecodingKey::from_ec_pem(key_bytes)
        .map_err(|e| ExecutorError::Jwt(format!("Invalid public key: {}", e)))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_audience(&["flow-like-executor"]);
    validation.set_issuer(&["flow-like"]);

    let token_data = decode::<ExecutorClaims>(token, &decoding_key, &validation)?;

    Ok(token_data.claims)
}

/// Verify and decode the executor JWT (sync version - requires BACKEND_PUB env var)
pub fn verify_jwt(token: &str) -> Result<ExecutorClaims, ExecutorError> {
    let public_key = std::env::var(BACKEND_PUB_ENV).map_err(|_| {
        ExecutorError::Config(format!(
            "{} not set (use verify_jwt_async for API fallback)",
            BACKEND_PUB_ENV
        ))
    })?;

    let key_bytes = STANDARD
        .decode(&public_key)
        .map_err(|e| ExecutorError::Jwt(format!("Failed to decode public key: {}", e)))?;

    let decoding_key = DecodingKey::from_ec_pem(&key_bytes)
        .map_err(|e| ExecutorError::Jwt(format!("Invalid public key: {}", e)))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_audience(&["flow-like-executor"]);
    validation.set_issuer(&["flow-like"]);

    let token_data = decode::<ExecutorClaims>(token, &decoding_key, &validation)?;

    Ok(token_data.claims)
}

/// Extract claims without verification (for extracting callback_url when verification is done elsewhere)
pub fn decode_claims_unverified(token: &str) -> Result<ExecutorClaims, ExecutorError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(ExecutorError::Jwt("Invalid JWT format".to_string()));
    }

    let payload = STANDARD
        .decode(parts[1])
        .map_err(|e| ExecutorError::Jwt(format!("Failed to decode payload: {}", e)))?;

    serde_json::from_slice(&payload)
        .map_err(|e| ExecutorError::Jwt(format!("Failed to parse claims: {}", e)))
}
