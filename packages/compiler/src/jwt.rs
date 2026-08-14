use base64::{engine::general_purpose::STANDARD, Engine};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use tokio::sync::OnceCell;

use crate::error::CompilerError;

pub const BACKEND_PUB_ENV: &str = "BACKEND_PUB";
pub const API_URL_ENV: &str = "API_BASE_URL";

static PUBLIC_KEY_CACHE: OnceCell<Vec<u8>> = OnceCell::const_new();
const JWKS_MAX_RESPONSE_BYTES: usize = 64 * 1024;
const JWKS_MAX_KEYS: usize = 8;

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
    #[serde(rename = "use")]
    key_use: String,
}

/// Claims embedded in the compiler JWT (signed by API).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerClaims {
    pub sub: String,
    pub job_id: String,
    pub package_id: String,
    pub version: String,
    pub payload_hash: String,
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

async fn fetch_public_key_from_api(expected_kid: &str) -> Result<Vec<u8>, CompilerError> {
    let api_url = std::env::var(API_URL_ENV).map_err(|_| {
        CompilerError::Config(format!(
            "Neither {BACKEND_PUB_ENV} nor {API_URL_ENV} is set"
        ))
    })?;

    let jwks_url = format!(
        "{}/execution/.well-known/jwks.json",
        api_url.trim_end_matches('/')
    );
    let jwks_url = reqwest::Url::parse(&jwks_url)
        .map_err(|_| CompilerError::Config("API_BASE_URL is invalid".to_string()))?;
    if !jwks_url.username().is_empty()
        || jwks_url.password().is_some()
        || jwks_url.query().is_some()
        || jwks_url.fragment().is_some()
        || !matches!(jwks_url.scheme(), "https" | "http")
        || (jwks_url.scheme() == "http" && !jwks_url.host_str().is_some_and(is_private_api_host))
    {
        return Err(CompilerError::Config(
            "API_BASE_URL must be HTTPS, or private HTTP for local development".to_string(),
        ));
    }

    tracing::info!("Fetching compiler verification key from the API");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| CompilerError::Config("failed to build JWKS client".to_string()))?;
    let mut response = client
        .get(jwks_url)
        .send()
        .await
        .map_err(|_| CompilerError::Config("failed to fetch compiler JWKS".to_string()))?;

    if !response.status().is_success() {
        return Err(CompilerError::Config(format!(
            "JWKS endpoint returned status {}",
            response.status()
        )));
    }

    if response
        .content_length()
        .is_some_and(|length| length > JWKS_MAX_RESPONSE_BYTES as u64)
    {
        return Err(CompilerError::Config(
            "compiler JWKS response is too large".to_string(),
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CompilerError::Config("failed to read compiler JWKS".to_string()))?
    {
        if body.len().saturating_add(chunk.len()) > JWKS_MAX_RESPONSE_BYTES {
            return Err(CompilerError::Config(
                "compiler JWKS response is too large".to_string(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    let jwks: Jwks = serde_json::from_slice(&body)
        .map_err(|_| CompilerError::Config("failed to parse compiler JWKS".to_string()))?;
    if jwks.keys.is_empty() || jwks.keys.len() > JWKS_MAX_KEYS {
        return Err(CompilerError::Config(
            "compiler JWKS contains an invalid number of keys".to_string(),
        ));
    }
    let mut matches = jwks.keys.iter().filter(|key| key.kid == expected_kid);
    let jwk = matches
        .next()
        .ok_or_else(|| CompilerError::Config("compiler JWT kid is not in JWKS".to_string()))?;
    if matches.next().is_some() {
        return Err(CompilerError::Config(
            "compiler JWKS contains duplicate kid values".to_string(),
        ));
    }

    if jwk.kty != "EC" || jwk.crv != "P-256" || jwk.alg != "ES256" || jwk.key_use != "sig" {
        return Err(CompilerError::Config(
            "unsupported compiler JWKS key metadata".to_string(),
        ));
    }

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let x_bytes = URL_SAFE_NO_PAD
        .decode(&jwk.x)
        .map_err(|_| CompilerError::Config("invalid compiler JWKS x coordinate".to_string()))?;
    let y_bytes = URL_SAFE_NO_PAD
        .decode(&jwk.y)
        .map_err(|_| CompilerError::Config("invalid compiler JWKS y coordinate".to_string()))?;

    let mut point = Vec::with_capacity(1 + x_bytes.len() + y_bytes.len());
    point.push(0x04);
    point.extend_from_slice(&x_bytes);
    point.extend_from_slice(&y_bytes);

    use p256::elliptic_curve::sec1::FromEncodedPoint;
    use p256::{EncodedPoint, PublicKey};

    let encoded_point = EncodedPoint::from_bytes(&point)
        .map_err(|_| CompilerError::Config("invalid compiler JWKS EC point".to_string()))?;

    let public_key = PublicKey::from_encoded_point(&encoded_point);
    if public_key.is_none().into() {
        return Err(CompilerError::Config(
            "invalid compiler JWKS public key".to_string(),
        ));
    }
    let public_key = public_key.unwrap();

    use p256::pkcs8::EncodePublicKey;
    let pem = public_key
        .to_public_key_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|_| CompilerError::Config("failed to encode compiler public key".to_string()))?;

    Ok(pem.into_bytes())
}

async fn get_public_key(expected_kid: &str) -> Result<&'static Vec<u8>, CompilerError> {
    PUBLIC_KEY_CACHE
        .get_or_try_init(|| async {
            if let Ok(b64) = std::env::var(BACKEND_PUB_ENV) {
                tracing::info!("Using {BACKEND_PUB_ENV} from environment");
                return STANDARD.decode(&b64).map_err(|_| {
                    CompilerError::Config(format!("Failed to decode {BACKEND_PUB_ENV}"))
                });
            }
            fetch_public_key_from_api(expected_kid).await
        })
        .await
}

pub async fn verify_jwt_async(token: &str) -> Result<CompilerClaims, CompilerError> {
    let header = decode_header(token)
        .map_err(|_| CompilerError::Jwt("invalid compiler JWT header".to_string()))?;
    if header.alg != Algorithm::ES256 {
        return Err(CompilerError::Jwt(
            "compiler JWT must use ES256".to_string(),
        ));
    }
    let kid = header
        .kid
        .as_deref()
        .filter(|kid| !kid.is_empty() && kid.len() <= 128)
        .ok_or_else(|| CompilerError::Jwt("compiler JWT must contain a valid kid".to_string()))?;
    if let Ok(expected_kid) = std::env::var("BACKEND_KID") {
        let expected_kid = expected_kid.trim();
        if !expected_kid.is_empty() && kid != expected_kid {
            return Err(CompilerError::Jwt(
                "compiler JWT kid does not match the configured backend key".to_string(),
            ));
        }
    }

    let key_bytes = get_public_key(kid).await?;

    let decoding_key = DecodingKey::from_ec_pem(key_bytes)
        .map_err(|_| CompilerError::Config("invalid configured compiler public key".to_string()))?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.leeway = 30;
    validation.set_required_spec_claims(&["exp", "nbf", "iat", "iss", "aud", "sub"]);
    validation.set_audience(&["flow-like-compiler"]);
    validation.set_issuer(&["flow-like"]);

    let token_data = decode::<CompilerClaims>(token, &decoding_key, &validation)?;
    let now = chrono::Utc::now().timestamp();
    if token_data.claims.iat > now + 60
        || token_data.claims.exp <= token_data.claims.iat
        || token_data.claims.exp - token_data.claims.iat > 24 * 60 * 60 + 60
    {
        return Err(CompilerError::Jwt(
            "compiler JWT lifetime is invalid".to_string(),
        ));
    }
    Ok(token_data.claims)
}

fn is_private_api_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || !host.contains('.') {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|address| match address {
        IpAddr::V4(address) => {
            address.is_private() || address.is_loopback() || address.is_link_local()
        }
        IpAddr::V6(address) => address.is_loopback() || address.is_unique_local(),
    })
}
