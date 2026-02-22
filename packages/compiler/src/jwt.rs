use base64::{engine::general_purpose::STANDARD, Engine};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::error::CompilerError;

pub const BACKEND_PUB_ENV: &str = "BACKEND_PUB";
pub const API_URL_ENV: &str = "API_BASE_URL";

static PUBLIC_KEY_CACHE: OnceCell<Vec<u8>> = OnceCell::const_new();

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
}

/// Claims embedded in the compiler JWT (signed by API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerClaims {
    pub sub: String,
    pub job_id: String,
    pub package_id: String,
    pub version: String,
    pub callback_url: String,
    #[serde(rename = "typ")]
    pub token_type: String,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

async fn fetch_public_key_from_api() -> Result<Vec<u8>, CompilerError> {
    let api_url = std::env::var(API_URL_ENV).map_err(|_| {
        CompilerError::Config(format!(
            "Neither {BACKEND_PUB_ENV} nor {API_URL_ENV} is set"
        ))
    })?;

    let jwks_url = format!(
        "{}/execution/.well-known/jwks.json",
        api_url.trim_end_matches('/')
    );

    tracing::info!(url = %jwks_url, "Fetching JWKS from API");

    let client = reqwest::Client::new();
    let response = client
        .get(&jwks_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| CompilerError::Jwt(format!("Failed to fetch JWKS: {e}")))?;

    if !response.status().is_success() {
        return Err(CompilerError::Jwt(format!(
            "JWKS endpoint returned status {}",
            response.status()
        )));
    }

    let jwks: Jwks = response
        .json()
        .await
        .map_err(|e| CompilerError::Jwt(format!("Failed to parse JWKS: {e}")))?;

    let jwk = jwks
        .keys
        .first()
        .ok_or_else(|| CompilerError::Jwt("JWKS contains no keys".to_string()))?;

    if jwk.kty != "EC" || jwk.crv != "P-256" {
        return Err(CompilerError::Jwt(format!(
            "Unsupported key type: {} {}",
            jwk.kty, jwk.crv
        )));
    }

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let x_bytes = URL_SAFE_NO_PAD
        .decode(&jwk.x)
        .map_err(|e| CompilerError::Jwt(format!("Failed to decode x: {e}")))?;
    let y_bytes = URL_SAFE_NO_PAD
        .decode(&jwk.y)
        .map_err(|e| CompilerError::Jwt(format!("Failed to decode y: {e}")))?;

    let mut point = Vec::with_capacity(1 + x_bytes.len() + y_bytes.len());
    point.push(0x04);
    point.extend_from_slice(&x_bytes);
    point.extend_from_slice(&y_bytes);

    use p256::elliptic_curve::sec1::FromEncodedPoint;
    use p256::{EncodedPoint, PublicKey};

    let encoded_point = EncodedPoint::from_bytes(&point)
        .map_err(|e| CompilerError::Jwt(format!("Invalid EC point: {e}")))?;

    let public_key = PublicKey::from_encoded_point(&encoded_point);
    if public_key.is_none().into() {
        return Err(CompilerError::Jwt("Invalid EC public key".to_string()));
    }
    let public_key = public_key.unwrap();

    use p256::pkcs8::EncodePublicKey;
    let pem = public_key
        .to_public_key_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| CompilerError::Jwt(format!("Failed to encode public key: {e}")))?;

    Ok(pem.into_bytes())
}

async fn get_public_key() -> Result<&'static Vec<u8>, CompilerError> {
    PUBLIC_KEY_CACHE
        .get_or_try_init(|| async {
            if let Ok(b64) = std::env::var(BACKEND_PUB_ENV) {
                tracing::info!("Using {BACKEND_PUB_ENV} from environment");
                return STANDARD
                    .decode(&b64)
                    .map_err(|e| CompilerError::Jwt(format!("Failed to decode {BACKEND_PUB_ENV}: {e}")));
            }
            fetch_public_key_from_api().await
        })
        .await
}

pub async fn verify_jwt_async(token: &str) -> Result<CompilerClaims, CompilerError> {
    let key_bytes = get_public_key().await?;

    let decoding_key = DecodingKey::from_ec_pem(key_bytes)
        .map_err(|e| CompilerError::Jwt(format!("Invalid public key: {e}")))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.set_audience(&["flow-like-compiler"]);
    validation.set_issuer(&["flow-like"]);

    let token_data = decode::<CompilerClaims>(token, &decoding_key, &validation)?;
    Ok(token_data.claims)
}
