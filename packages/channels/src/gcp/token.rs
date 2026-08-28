//! Firebase custom tokens: the RS256 JWT a client or the executor exchanges for an ID token.

use flow_like_types::anyhow;
use flow_like_types::channel::now_unix;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use serde_json::Value;

pub const FIREBASE_CUSTOM_TOKEN_AUDIENCE: &str =
    "https://identitytoolkit.googleapis.com/google.identity.identitytoolkit.v1.IdentityToolkit";
pub const MAX_CUSTOM_TOKEN_TTL_SECS: i64 = 3600;
pub const MAX_CUSTOM_CLAIMS_BYTES: usize = 1000;
pub const MAX_UID_CHARS: usize = 128;
/// Claim names Firebase refuses inside `claims`: OIDC-reserved plus its own.
pub const RESERVED_CLAIMS: &[&str] = &[
    "acr",
    "amr",
    "at_hash",
    "aud",
    "auth_time",
    "azp",
    "cnf",
    "c_hash",
    "exp",
    "iat",
    "iss",
    "jti",
    "nbf",
    "nonce",
    "sub",
    "firebase",
    "user_id",
];

#[derive(Serialize)]
struct CustomTokenClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'static str,
    iat: i64,
    exp: i64,
    uid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    claims: Option<Value>,
}

/// Mint a custom token for `uid` carrying `claims` (a JSON object, or `null` for none), signed
/// with the service account's `private_key` (PKCS#8 `PRIVATE KEY` PEM as shipped in the
/// service-account JSON; PKCS#1 `RSA PRIVATE KEY` is accepted too). `ttl_secs` is clamped to
/// Firebase's one-hour maximum.
pub fn custom_token(
    service_account_email: &str,
    private_key_pem: &str,
    key_id: Option<&str>,
    uid: &str,
    claims: Value,
    ttl_secs: i64,
) -> flow_like_types::Result<String> {
    if service_account_email.trim().is_empty() {
        return Err(anyhow!(
            "firebase custom token: service account email is empty"
        ));
    }
    validate_uid(uid)?;
    let claims = validate_claims(claims)?;
    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes()).map_err(|err| {
        anyhow!("firebase custom token: service account private key is not an RSA PEM key: {err}")
    })?;
    let iat = now_unix();
    let exp = iat + ttl_secs.clamp(1, MAX_CUSTOM_TOKEN_TTL_SECS);
    let mut header = Header::new(Algorithm::RS256);
    header.kid = key_id.filter(|id| !id.is_empty()).map(str::to_string);
    encode(
        &header,
        &CustomTokenClaims {
            iss: service_account_email,
            sub: service_account_email,
            aud: FIREBASE_CUSTOM_TOKEN_AUDIENCE,
            iat,
            exp,
            uid,
            claims,
        },
        &key,
    )
    .map_err(|err| anyhow!("firebase custom token: signing failed: {err}"))
}

fn validate_uid(uid: &str) -> flow_like_types::Result<()> {
    let chars = uid.chars().count();
    if chars == 0 || chars > MAX_UID_CHARS {
        return Err(anyhow!(
            "firebase custom token: uid must be 1-{MAX_UID_CHARS} characters, got {chars}"
        ));
    }
    Ok(())
}

fn validate_claims(claims: Value) -> flow_like_types::Result<Option<Value>> {
    let map = match claims {
        Value::Null => return Ok(None),
        Value::Object(map) => map,
        other => {
            return Err(anyhow!(
                "firebase custom token: claims must be a JSON object, got {other}"
            ));
        }
    };
    if map.is_empty() {
        return Ok(None);
    }
    if let Some(reserved) = map
        .keys()
        .find(|key| RESERVED_CLAIMS.contains(&key.as_str()))
    {
        return Err(anyhow!(
            "firebase custom token: claim name '{reserved}' is reserved"
        ));
    }
    let claims = Value::Object(map);
    let size = serde_json::to_vec(&claims)?.len();
    if size > MAX_CUSTOM_CLAIMS_BYTES {
        return Err(anyhow!(
            "firebase custom token: claims serialize to {size} bytes, the limit is {MAX_CUSTOM_CLAIMS_BYTES}"
        ));
    }
    Ok(Some(claims))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcp::{EXECUTOR_UID, client_claims, executor_claims};
    use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
    use rsa::{RsaPrivateKey, RsaPublicKey};
    use serde_json::json;

    const EMAIL: &str = "channels@test-project.iam.gserviceaccount.com";

    fn throwaway_key() -> (String, String, String) {
        let mut rng = rand::thread_rng();
        let key = RsaPrivateKey::new(&mut rng, 1024).expect("throwaway key");
        let pkcs8 = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let pkcs1 = key.to_pkcs1_pem(LineEnding::LF).unwrap().to_string();
        let public = RsaPublicKey::from(&key)
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        (pkcs8, pkcs1, public)
    }

    fn verify(token: &str, public_pem: &str) -> Value {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[FIREBASE_CUSTOM_TOKEN_AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "aud"]);
        decode::<Value>(
            token,
            &DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap(),
            &validation,
        )
        .expect("token must verify")
        .claims
    }

    #[test]
    fn custom_token_roundtrips_through_the_public_key() {
        let (pkcs8, pkcs1, public) = throwaway_key();
        assert!(pkcs8.starts_with("-----BEGIN PRIVATE KEY-----"));

        let token = custom_token(
            EMAIL,
            &pkcs8,
            Some("kid-1"),
            "user-42",
            client_claims("run-1"),
            7200,
        )
        .unwrap();
        let header = decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::RS256);
        assert_eq!(header.kid.as_deref(), Some("kid-1"));
        let claims = verify(&token, &public);
        assert_eq!(claims["iss"], EMAIL);
        assert_eq!(claims["sub"], EMAIL);
        assert_eq!(claims["aud"], FIREBASE_CUSTOM_TOKEN_AUDIENCE);
        assert_eq!(claims["uid"], "user-42");
        assert_eq!(
            claims["claims"],
            json!({ "run_id": "run-1", "role": "client" })
        );
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            MAX_CUSTOM_TOKEN_TTL_SECS
        );

        let token = custom_token(
            EMAIL,
            &pkcs1,
            None,
            EXECUTOR_UID,
            executor_claims("run-1"),
            600,
        )
        .unwrap();
        assert!(decode_header(&token).unwrap().kid.is_none());
        let claims = verify(&token, &public);
        assert_eq!(claims["uid"], "svc");
        assert_eq!(claims["claims"]["role"], "server");
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            600
        );

        let token = custom_token(EMAIL, &pkcs8, None, "u", Value::Null, 60).unwrap();
        assert!(verify(&token, &public).get("claims").is_none());
        let token = custom_token(EMAIL, &pkcs8, None, "u", json!({}), 60).unwrap();
        assert!(verify(&token, &public).get("claims").is_none());
    }

    #[test]
    fn rejects_reserved_and_oversized_claims() {
        for reserved in RESERVED_CLAIMS {
            let err = custom_token(EMAIL, "", None, "u", json!({ *reserved: 1 }), 60).unwrap_err();
            assert!(err.to_string().contains("reserved"), "{reserved}: {err}");
        }
        let err = custom_token(
            EMAIL,
            "",
            None,
            "u",
            json!({ "run_id": "x".repeat(MAX_CUSTOM_CLAIMS_BYTES) }),
            60,
        )
        .unwrap_err();
        assert!(err.to_string().contains("bytes"), "{err}");
        let err =
            custom_token(EMAIL, "", None, "u", json!(["not", "an", "object"]), 60).unwrap_err();
        assert!(err.to_string().contains("JSON object"), "{err}");
    }

    #[test]
    fn rejects_bad_uid_email_and_key() {
        assert!(custom_token(EMAIL, "", None, "", Value::Null, 60).is_err());
        assert!(custom_token(EMAIL, "", None, &"u".repeat(129), Value::Null, 60).is_err());
        assert!(custom_token(" ", "", None, "u", Value::Null, 60).is_err());
        let err =
            custom_token(EMAIL, "-----BEGIN NOPE-----", None, "u", Value::Null, 60).unwrap_err();
        assert!(err.to_string().contains("RSA PEM"), "{err}");
    }
}
