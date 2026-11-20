//! Only the API may call the service: every request carries the API's ES256 token, bound to the
//! exact body by its SHA-256. The verification key is the API's public key, `BACKEND_PUB`.

use crate::protocol::{AUDIENCE, ErrorCode, JOSE_TYPE, QueryClaims, WireError};
use base64::Engine;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

const ISSUER: &str = "flow-like";
const MAX_LIFETIME_SECONDS: i64 = 300;
const LEEWAY_SECONDS: u64 = 30;
pub const BACKEND_PUB_ENV: &str = "BACKEND_PUB";

pub struct TokenVerifier {
    key: DecodingKey,
}

fn unauthorized(message: impl Into<String>) -> WireError {
    WireError::new(ErrorCode::Unauthorized, message)
}

impl TokenVerifier {
    pub fn from_public_pem(pem: &[u8]) -> Result<Self, String> {
        Ok(Self {
            key: DecodingKey::from_ec_pem(pem)
                .map_err(|error| format!("The API public key is not an EC PEM key: {error}"))?,
        })
    }

    /// The key from `BACKEND_PUB`: base64 of the PEM, as the executor reads it.
    pub fn from_env() -> Result<Self, String> {
        let encoded = std::env::var(BACKEND_PUB_ENV).map_err(|_| {
            format!("{BACKEND_PUB_ENV} is not set; the service accepts no requests")
        })?;
        let pem = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|error| format!("{BACKEND_PUB_ENV} is not valid base64: {error}"))?;
        Self::from_public_pem(&pem)
    }

    pub fn verify(&self, token: &str, body_sha256: &str) -> Result<QueryClaims, WireError> {
        if token.len() > 16_384 {
            return Err(unauthorized("The request token is too long"));
        }
        let header =
            decode_header(token).map_err(|_| unauthorized("The request token is malformed"))?;
        if header.alg != Algorithm::ES256 || header.typ.as_deref() != Some(JOSE_TYPE) {
            return Err(unauthorized(
                "The request token is not a query service token",
            ));
        }
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp", "nbf", "iat"]);
        validation.validate_nbf = true;
        validation.leeway = LEEWAY_SECONDS;
        let claims = decode::<QueryClaims>(token, &self.key, &validation)
            .map_err(|error| unauthorized(format!("The request token was refused: {error}")))?
            .claims;
        if claims.exp - claims.iat > MAX_LIFETIME_SECONDS {
            return Err(unauthorized(
                "The request token lives longer than the service allows",
            ));
        }
        if claims.body_sha256 != body_sha256 {
            return Err(unauthorized(
                "The request token was issued for another request body",
            ));
        }
        Ok(claims)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::protocol::body_digest;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use p256::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};

    pub(crate) struct TestKeys {
        pub private_pem: String,
        pub public_pem: String,
    }

    pub(crate) fn test_keys() -> TestKeys {
        let secret = loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).unwrap();
            if let Ok(key) = p256::SecretKey::from_slice(&bytes) {
                break key;
            }
        };
        TestKeys {
            private_pem: secret.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
            public_pem: secret
                .public_key()
                .to_public_key_pem(LineEnding::LF)
                .unwrap(),
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn claims(body: &[u8]) -> QueryClaims {
        let now = now();
        QueryClaims {
            iss: ISSUER.to_string(),
            sub: "user-1".to_string(),
            aud: AUDIENCE.to_string(),
            iat: now,
            nbf: now,
            exp: now + 60,
            jti: "jti-1".to_string(),
            request_id: "request-1".to_string(),
            body_sha256: body_digest(body),
        }
    }

    pub(crate) fn sign(private_pem: &str, typ: &str, claims: &QueryClaims) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some(typ.to_string());
        encode(
            &header,
            claims,
            &EncodingKey::from_ec_pem(private_pem.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn accepts_only_an_api_token_bound_to_this_body() {
        let keys = test_keys();
        let verifier = TokenVerifier::from_public_pem(keys.public_pem.as_bytes()).unwrap();
        let body = br#"{"sql":"SELECT 1"}"#;
        let digest = body_digest(body);
        let valid = claims(body);

        assert!(
            verifier
                .verify(&sign(&keys.private_pem, JOSE_TYPE, &valid), &digest)
                .is_ok()
        );
        assert!(
            verifier
                .verify(
                    &sign(&keys.private_pem, JOSE_TYPE, &valid),
                    &body_digest(b"other")
                )
                .is_err(),
            "a token is bound to its body"
        );
        assert!(
            verifier
                .verify(&sign(&keys.private_pem, "JWT", &valid), &digest)
                .is_err(),
            "other API token types are refused"
        );
        let mut foreign = valid.clone();
        foreign.aud = "flow-like-executor".to_string();
        assert!(
            verifier
                .verify(&sign(&keys.private_pem, JOSE_TYPE, &foreign), &digest)
                .is_err()
        );
        let mut expired = valid.clone();
        expired.iat -= 600;
        expired.nbf -= 600;
        expired.exp = expired.iat + 60;
        assert!(
            verifier
                .verify(&sign(&keys.private_pem, JOSE_TYPE, &expired), &digest)
                .is_err()
        );
        let mut long_lived = valid.clone();
        long_lived.exp = long_lived.iat + 3_600;
        assert!(
            verifier
                .verify(&sign(&keys.private_pem, JOSE_TYPE, &long_lived), &digest)
                .is_err()
        );
        let other_keys = test_keys();
        assert!(
            verifier
                .verify(&sign(&other_keys.private_pem, JOSE_TYPE, &valid), &digest)
                .is_err(),
            "tokens of another key are refused"
        );
    }
}
