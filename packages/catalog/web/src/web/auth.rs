use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flow_like_types::json::{self, json};
use flow_like_types::utils::constant_time_eq;
use flow_like_types::{Value, anyhow};
use hmac::{Hmac, Mac};
use jsonwebtoken::jwk::{Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::str::FromStr;

use super::http_runtime::{HttpRequest, HttpResponse};
use super::rest::RestAuthConfig;

#[derive(Clone)]
pub(crate) struct OAuthValidator {
    issuer: Option<String>,
    audience: Option<String>,
    required_scopes: Vec<String>,
    jwks: JwkSet,
}

pub(crate) async fn build_oauth_validator(
    context: &mut ExecutionContext,
    auth: &RestAuthConfig,
) -> flow_like_types::Result<Option<OAuthValidator>> {
    let RestAuthConfig::OAuthBearer {
        issuer,
        audience,
        required_scopes,
        jwks_url,
        jwks_flow_path,
        oidc_discovery_url,
    } = auth
    else {
        return Ok(None);
    };

    let mut issuer = clean_optional(issuer.clone());
    let jwks_url = jwks_url
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let oidc_discovery_url = oidc_discovery_url
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty());
    let source_count = jwks_url.is_some() as u8
        + jwks_flow_path.is_some() as u8
        + oidc_discovery_url.is_some() as u8;
    if source_count > 1 {
        return Err(anyhow!(
            "OAuth auth config must use only one of jwks_url, jwks_flow_path, or oidc_discovery_url"
        ));
    }

    let jwks_bytes = match (jwks_url, jwks_flow_path.as_ref(), oidc_discovery_url) {
        (Some(url), None, None) => fetch_jwks(url).await?,
        (None, Some(flow_path), None) => flow_path.get(context, false).await?,
        (None, None, Some(discovery_url)) => {
            let discovery = fetch_oidc_discovery(discovery_url).await?;
            if issuer.is_none() {
                issuer = discovery.issuer;
            }
            fetch_jwks(&discovery.jwks_uri).await?
        }
        (None, None, None) => {
            return Err(anyhow!(
                "OAuth auth config requires a jwks_url, jwks_flow_path, or oidc_discovery_url"
            ));
        }
        _ => unreachable!("OAuth source_count prevents multiple JWKS sources"),
    };

    let jwks = json::from_slice::<JwkSet>(&jwks_bytes)
        .map_err(|err| anyhow!("Failed to parse OAuth JWKS: {}", err))?;

    Ok(Some(OAuthValidator::new(
        issuer,
        audience.clone().filter(|value| !value.is_empty()),
        required_scopes.clone(),
        jwks,
    )))
}

pub(crate) fn authorize_client(
    auth: &RestAuthConfig,
    oauth_validator: Option<&OAuthValidator>,
    request: &HttpRequest,
    protocol: &str,
) -> Result<Value, HttpResponse> {
    match auth {
        RestAuthConfig::None => Ok(client_metadata(request, protocol, None)),
        RestAuthConfig::ApiKey { header, key } => {
            let actual = request.headers.get(&header.to_lowercase());
            if actual.is_some_and(|actual| constant_time_eq(actual.as_bytes(), key.as_bytes())) {
                Ok(client_metadata(request, protocol, None))
            } else {
                Err(HttpResponse::text(401, "Unauthorized"))
            }
        }
        RestAuthConfig::BearerToken { token } => {
            let expected = format!("Bearer {}", token);
            let actual = request.headers.get("authorization");
            if actual.is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected.as_bytes()))
            {
                Ok(client_metadata(request, protocol, None))
            } else {
                Err(HttpResponse::text(401, "Unauthorized"))
            }
        }
        RestAuthConfig::BasicAuth { username, password } => {
            let expected = basic_auth_header(username, password);
            let actual = request.headers.get("authorization");
            if actual.is_some_and(|actual| constant_time_eq(actual.as_bytes(), expected.as_bytes()))
            {
                Ok(client_metadata(request, protocol, None))
            } else {
                Err(HttpResponse::text(401, "Unauthorized"))
            }
        }
        RestAuthConfig::HmacSha256 {
            secret,
            signature_header,
            timestamp_header,
            max_skew_seconds,
        } => match validate_hmac_request(
            request,
            secret,
            signature_header,
            timestamp_header,
            *max_skew_seconds,
        ) {
            Ok(()) => Ok(client_metadata(request, protocol, None)),
            Err(AuthError::Unauthorized(message)) => Err(HttpResponse::text(401, message)),
            Err(AuthError::Forbidden(message)) => Err(HttpResponse::text(403, message)),
        },
        RestAuthConfig::OAuthBearer { .. } => {
            let Some(validator) = oauth_validator else {
                return Err(HttpResponse::text(
                    500,
                    "OAuth validator was not initialized",
                ));
            };
            match validator.validate_request(request) {
                Ok(claims) => Ok(client_metadata(request, protocol, Some(claims))),
                Err(AuthError::Unauthorized(message)) => Err(HttpResponse::text(401, message)),
                Err(AuthError::Forbidden(message)) => Err(HttpResponse::text(403, message)),
            }
        }
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn client_metadata(
    request: &HttpRequest,
    protocol: &str,
    oauth_claims: Option<Value>,
) -> Value {
    let mut client = json::Map::new();
    client.insert("remote_addr".to_string(), json!(request.remote_addr));
    client.insert("protocol".to_string(), json!(protocol));

    if let Some(claims) = oauth_claims {
        if let Some(value) = claims.get("sub").cloned() {
            client.insert("sub".to_string(), value);
        }
        if let Some(value) = claims.get("iss").cloned() {
            client.insert("issuer".to_string(), value);
        }
        if let Some(value) = claims.get("aud").cloned() {
            client.insert("audience".to_string(), value);
        }
        if let Some(value) = claims
            .get("client_id")
            .or_else(|| claims.get("azp"))
            .cloned()
        {
            client.insert("client_id".to_string(), value);
        }
        if let Some(value) = claims.get("email").cloned() {
            client.insert("email".to_string(), value);
        }
        let scopes = scopes_from_claims(&claims);
        if !scopes.is_empty() {
            client.insert(
                "scopes".to_string(),
                Value::Array(scopes.into_iter().map(Value::String).collect()),
            );
        }
        client.insert(
            "auth".to_string(),
            json!({
                "type": "oauth_bearer",
                "claims": claims
            }),
        );
    }

    Value::Object(client)
}

pub(crate) fn payload_with_client(payload: Value, client: &Value) -> Value {
    match payload {
        Value::Object(mut object) => {
            object.insert("_client".to_string(), client.clone());
            Value::Object(object)
        }
        other => json!({
            "payload": other,
            "_client": client
        }),
    }
}

impl OAuthValidator {
    pub(crate) fn new(
        issuer: Option<String>,
        audience: Option<String>,
        required_scopes: Vec<String>,
        jwks: JwkSet,
    ) -> Self {
        Self {
            issuer,
            audience,
            required_scopes: required_scopes
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect(),
            jwks,
        }
    }

    fn validate_request(&self, request: &HttpRequest) -> Result<Value, AuthError> {
        let token = bearer_token(request)?;
        self.validate_token(token)
    }

    fn validate_token(&self, token: &str) -> Result<Value, AuthError> {
        let header = decode_header(token)
            .map_err(|err| AuthError::Unauthorized(format!("Invalid token header: {}", err)))?;
        if !is_asymmetric_algorithm(header.alg) {
            return Err(AuthError::Unauthorized(
                "Unsupported OAuth token algorithm".to_string(),
            ));
        }

        let mut validation = Validation::new(header.alg);
        validation.set_required_spec_claims(&["exp"]);
        validation.validate_nbf = true;
        if let Some(issuer) = &self.issuer {
            validation.set_issuer(&[issuer]);
            validation.required_spec_claims.insert("iss".to_string());
        }
        if let Some(audience) = &self.audience {
            validation.set_audience(&[audience]);
            validation.required_spec_claims.insert("aud".to_string());
        } else {
            validation.validate_aud = false;
        }

        let candidates = candidate_keys(&self.jwks, header.kid.as_deref());
        if candidates.is_empty() {
            return Err(AuthError::Unauthorized(
                "No matching OAuth JWKS key found".to_string(),
            ));
        }

        let mut last_error = None;
        for jwk in candidates {
            if !jwk_matches_header(jwk, header.alg) {
                continue;
            }
            let key = match DecodingKey::from_jwk(jwk) {
                Ok(key) => key,
                Err(err) => {
                    last_error = Some(err.to_string());
                    continue;
                }
            };
            match decode::<Value>(token, &key, &validation) {
                Ok(data) => {
                    self.validate_scopes(&data.claims)?;
                    return Ok(data.claims);
                }
                Err(err) => last_error = Some(err.to_string()),
            }
        }

        Err(AuthError::Unauthorized(format!(
            "Invalid OAuth bearer token: {}",
            last_error.unwrap_or_else(|| "no usable JWKS key".to_string())
        )))
    }

    fn validate_scopes(&self, claims: &Value) -> Result<(), AuthError> {
        if self.required_scopes.is_empty() {
            return Ok(());
        }

        let scopes = scopes_from_claims(claims)
            .into_iter()
            .collect::<HashSet<_>>();
        for required in &self.required_scopes {
            if !scopes.contains(required) {
                return Err(AuthError::Forbidden(format!(
                    "Missing required OAuth scope: {}",
                    required
                )));
            }
        }
        Ok(())
    }
}

async fn fetch_jwks(url: &str) -> flow_like_types::Result<Vec<u8>> {
    let response = reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| anyhow!("Failed to fetch OAuth JWKS: {}", err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("OAuth JWKS endpoint returned {}", status));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|err| anyhow!("Failed to read OAuth JWKS response: {}", err))
}

struct OidcDiscovery {
    issuer: Option<String>,
    jwks_uri: String,
}

async fn fetch_oidc_discovery(url: &str) -> flow_like_types::Result<OidcDiscovery> {
    let response = reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| anyhow!("Failed to fetch OIDC discovery document: {}", err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("OIDC discovery endpoint returned {}", status));
    }
    let document = response
        .json::<Value>()
        .await
        .map_err(|err| anyhow!("Failed to parse OIDC discovery document: {}", err))?;
    oidc_discovery_from_document(&document)
}

fn oidc_discovery_from_document(document: &Value) -> flow_like_types::Result<OidcDiscovery> {
    let jwks_uri = document
        .get("jwks_uri")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("OIDC discovery document is missing jwks_uri"))?
        .to_string();
    let issuer = document
        .get("issuer")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    Ok(OidcDiscovery { issuer, jwks_uri })
}

fn bearer_token(request: &HttpRequest) -> Result<&str, AuthError> {
    let header = request
        .headers
        .get("authorization")
        .ok_or_else(|| AuthError::Unauthorized("Missing bearer token".to_string()))?;
    let Some(token) = header.strip_prefix("Bearer ") else {
        return Err(AuthError::Unauthorized("Missing bearer token".to_string()));
    };
    let token = token.trim();
    if token.is_empty() {
        return Err(AuthError::Unauthorized("Missing bearer token".to_string()));
    }
    Ok(token)
}

fn basic_auth_header(username: &str, password: &str) -> String {
    format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("{}:{}", username, password))
    )
}

fn validate_hmac_request(
    request: &HttpRequest,
    secret: &str,
    signature_header: &str,
    timestamp_header: &str,
    max_skew_seconds: u64,
) -> Result<(), AuthError> {
    let signature_header = signature_header.trim().to_lowercase();
    let timestamp_header = timestamp_header.trim().to_lowercase();
    if signature_header.is_empty() || timestamp_header.is_empty() {
        return Err(AuthError::Unauthorized(
            "HMAC auth requires signature and timestamp headers".to_string(),
        ));
    }

    let signature = request
        .headers
        .get(&signature_header)
        .ok_or_else(|| AuthError::Unauthorized("Missing HMAC signature".to_string()))?;
    let timestamp = request
        .headers
        .get(&timestamp_header)
        .ok_or_else(|| AuthError::Unauthorized("Missing HMAC timestamp".to_string()))?;
    if max_skew_seconds > 0 {
        validate_hmac_timestamp(timestamp, max_skew_seconds)?;
    }

    let expected = hmac_signature(secret, request, timestamp)
        .map_err(|err| AuthError::Unauthorized(err.to_string()))?;
    let actual = decode_hex_signature(signature)
        .ok_or_else(|| AuthError::Unauthorized("Invalid HMAC signature encoding".to_string()))?;
    expected
        .verify_slice(&actual)
        .map_err(|_| AuthError::Unauthorized("Invalid HMAC signature".to_string()))
}

fn validate_hmac_timestamp(timestamp: &str, max_skew_seconds: u64) -> Result<(), AuthError> {
    let timestamp = timestamp
        .trim()
        .parse::<i64>()
        .map_err(|_| AuthError::Unauthorized("Invalid HMAC timestamp".to_string()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| AuthError::Unauthorized("Invalid system clock".to_string()))?
        .as_secs() as i64;
    if now.abs_diff(timestamp) > max_skew_seconds {
        return Err(AuthError::Unauthorized("Stale HMAC timestamp".to_string()));
    }
    Ok(())
}

fn hmac_signature(
    secret: &str,
    request: &HttpRequest,
    timestamp: &str,
) -> flow_like_types::Result<Hmac<Sha256>> {
    let canonical = hmac_canonical_string(request, timestamp);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|err| anyhow!("Failed to initialize HMAC: {}", err))?;
    mac.update(canonical.as_bytes());
    Ok(mac)
}

fn hmac_canonical_string(request: &HttpRequest, timestamp: &str) -> String {
    format!(
        "{}\n{}\n{}\n{}",
        request.method.to_uppercase(),
        request.path,
        timestamp.trim(),
        sha256_hex(&request.body)
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    lower_hex(&Sha256::digest(bytes))
}

fn decode_hex_signature(signature: &str) -> Option<Vec<u8>> {
    let signature = signature
        .trim()
        .strip_prefix("sha256=")
        .unwrap_or(signature.trim());
    if !signature.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(signature.len() / 2);
    for pair in signature.as_bytes().chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn candidate_keys<'a>(jwks: &'a JwkSet, kid: Option<&str>) -> Vec<&'a Jwk> {
    if let Some(kid) = kid {
        jwks.find(kid).into_iter().collect()
    } else {
        jwks.keys.iter().collect()
    }
}

fn jwk_matches_header(jwk: &Jwk, alg: Algorithm) -> bool {
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| usage != &PublicKeyUse::Signature)
    {
        return false;
    }
    if jwk.common.key_operations.as_ref().is_some_and(|ops| {
        !ops.iter()
            .any(|op| matches!(op, KeyOperations::Verify | KeyOperations::Sign))
    }) {
        return false;
    }
    if let Some(key_alg) = &jwk.common.key_algorithm {
        return key_algorithm_matches(key_alg, alg);
    }
    true
}

fn key_algorithm_matches(key_alg: &KeyAlgorithm, alg: Algorithm) -> bool {
    let Ok(jwk_alg) = Algorithm::from_str(&key_alg.to_string()) else {
        return false;
    };
    jwk_alg == alg
}

fn is_asymmetric_algorithm(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

fn scopes_from_claims(claims: &Value) -> Vec<String> {
    let mut scopes = Vec::new();
    if let Some(scope) = claims.get("scope").and_then(|value| value.as_str()) {
        scopes.extend(scope.split_whitespace().map(ToString::to_string));
    }
    if let Some(scope) = claims.get("scp").and_then(|value| value.as_str()) {
        scopes.extend(scope.split_whitespace().map(ToString::to_string));
    }
    for key in ["scp", "permissions"] {
        if let Some(values) = claims.get(key).and_then(|value| value.as_array()) {
            scopes.extend(
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToString::to_string)),
            );
        }
    }
    scopes.sort();
    scopes.dedup();
    scopes
}

#[derive(Debug)]
enum AuthError {
    Unauthorized(String),
    Forbidden(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::jwk::Jwk;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use rcgen::{KeyPair, PKCS_ECDSA_P256_SHA256};
    use std::collections::HashMap;

    #[test]
    fn oauth_validator_accepts_es256_jwks_and_returns_claims() {
        let (token, jwks) = test_token_and_jwks("read write");
        let validator = OAuthValidator::new(
            Some("https://issuer.example".to_string()),
            Some("flow-like".to_string()),
            vec!["read".to_string()],
            jwks,
        );

        let claims = validator.validate_token(&token).unwrap();

        assert_eq!(claims["sub"], "user-1");
        assert_eq!(claims["scope"], "read write");
    }

    #[test]
    fn oauth_validator_rejects_missing_scope() {
        let (token, jwks) = test_token_and_jwks("read");
        let validator = OAuthValidator::new(
            Some("https://issuer.example".to_string()),
            Some("flow-like".to_string()),
            vec!["admin".to_string()],
            jwks,
        );

        let err = validator.validate_token(&token).unwrap_err();

        assert!(matches!(err, AuthError::Forbidden(_)));
    }

    #[test]
    fn api_key_auth_accepts_valid_and_rejects_invalid_or_missing_credentials() {
        let auth = RestAuthConfig::ApiKey {
            header: "X-Flow-Key".to_string(),
            key: "runtime-secret".to_string(),
        };

        let mut valid = test_request("GET", "/secure", Vec::new());
        valid
            .headers
            .insert("x-flow-key".to_string(), "runtime-secret".to_string());
        assert!(authorize_client(&auth, None, &valid, "rest").is_ok());

        let mut invalid = test_request("GET", "/secure", Vec::new());
        invalid
            .headers
            .insert("x-flow-key".to_string(), "runtime-secrex".to_string());
        assert_unauthorized(authorize_client(&auth, None, &invalid, "rest"));

        let missing = test_request("GET", "/secure", Vec::new());
        assert_unauthorized(authorize_client(&auth, None, &missing, "rest"));
    }

    #[test]
    fn bearer_auth_accepts_valid_and_rejects_invalid_or_missing_credentials() {
        let auth = RestAuthConfig::BearerToken {
            token: "runtime-secret".to_string(),
        };

        let mut valid = test_request("GET", "/secure", Vec::new());
        valid.headers.insert(
            "authorization".to_string(),
            "Bearer runtime-secret".to_string(),
        );
        assert!(authorize_client(&auth, None, &valid, "rest").is_ok());

        let mut invalid = test_request("GET", "/secure", Vec::new());
        invalid.headers.insert(
            "authorization".to_string(),
            "Bearer runtime-secrex".to_string(),
        );
        assert_unauthorized(authorize_client(&auth, None, &invalid, "rest"));

        let missing = test_request("GET", "/secure", Vec::new());
        assert_unauthorized(authorize_client(&auth, None, &missing, "rest"));

        let mut lowercase_scheme = test_request("GET", "/secure", Vec::new());
        lowercase_scheme.headers.insert(
            "authorization".to_string(),
            "bearer runtime-secret".to_string(),
        );
        assert_unauthorized(authorize_client(&auth, None, &lowercase_scheme, "rest"));
    }

    #[test]
    fn basic_auth_accepts_valid_and_rejects_invalid_or_missing_credentials() {
        let auth = RestAuthConfig::BasicAuth {
            username: "flow".to_string(),
            password: "secret".to_string(),
        };

        let mut request = test_request("GET", "/secure", Vec::new());
        request.headers.insert(
            "authorization".to_string(),
            basic_auth_header("flow", "secret"),
        );
        let client = authorize_client(&auth, None, &request, "rest").unwrap();

        assert_eq!(client["protocol"], "rest");

        request.headers.insert(
            "authorization".to_string(),
            basic_auth_header("flow", "secrex"),
        );
        assert_unauthorized(authorize_client(&auth, None, &request, "rest"));

        request.headers.remove("authorization");
        assert_unauthorized(authorize_client(&auth, None, &request, "rest"));
    }

    #[test]
    fn hmac_auth_accepts_expected_signature() {
        let mut request = test_request("POST", "/signed", b"{\"ok\":true}".to_vec());
        let timestamp = jsonwebtoken::get_current_timestamp().to_string();
        request
            .headers
            .insert("x-timestamp".to_string(), timestamp.clone());
        let signature = lower_hex(
            &hmac_signature("secret", &request, &timestamp)
                .unwrap()
                .finalize()
                .into_bytes(),
        );
        request.headers.insert("x-signature".to_string(), signature);

        let client = authorize_client(
            &RestAuthConfig::HmacSha256 {
                secret: "secret".to_string(),
                signature_header: "x-signature".to_string(),
                timestamp_header: "x-timestamp".to_string(),
                max_skew_seconds: 300,
            },
            None,
            &request,
            "rest",
        )
        .unwrap();

        assert_eq!(client["protocol"], "rest");
    }

    #[test]
    fn oidc_discovery_document_extracts_jwks_uri_and_issuer() {
        let discovery = oidc_discovery_from_document(&json!({
            "issuer": "https://issuer.example",
            "jwks_uri": "https://issuer.example/keys"
        }))
        .unwrap();

        assert_eq!(discovery.issuer.as_deref(), Some("https://issuer.example"));
        assert_eq!(discovery.jwks_uri, "https://issuer.example/keys");
    }

    fn test_token_and_jwks(scope: &str) -> (String, JwkSet) {
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let private_pem = key_pair.serialize_pem();
        let encoding_key = EncodingKey::from_ec_pem(private_pem.as_bytes()).unwrap();
        let mut jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256).unwrap();
        jwk.common.key_id = Some("test-key".to_string());

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_string());
        let claims = json!({
            "sub": "user-1",
            "iss": "https://issuer.example",
            "aud": "flow-like",
            "scope": scope,
            "exp": jsonwebtoken::get_current_timestamp() + 3600,
            "nbf": jsonwebtoken::get_current_timestamp() - 60
        });
        let token = encode(&header, &claims, &encoding_key).unwrap();
        (token, JwkSet { keys: vec![jwk] })
    }

    fn test_request(method: &str, path: &str, body: Vec<u8>) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            query: HashMap::new(),
            headers: HashMap::new(),
            body,
            remote_addr: "127.0.0.1:1234".to_string(),
        }
    }

    fn assert_unauthorized(result: Result<Value, HttpResponse>) {
        let response = result.expect_err("authorization should fail");
        assert_eq!(response.status_code, 401);
    }
}
