use crate::backend_jwt::{self, BackendJwtError, TokenType};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

pub const ENROLLMENT_JOSE_TYPE: &str = "flow-like-device-enrollment+jwt";
pub const SESSION_JOSE_TYPE: &str = "flow-like-device-session+jwt";
pub const SESSION_SECONDS: i64 = 600;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    pub jkt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentClaims {
    pub sub: String,
    pub enrollment_id: String,
    pub device_id: String,
    pub cnf: Confirmation,
    pub scope: String,
    pub typ: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionClaims {
    pub sub: String,
    pub device_id: String,
    pub owner_id: String,
    pub auth_epoch: u64,
    pub cnf: Confirmation,
    pub scope: String,
    pub typ: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

fn invalid() -> BackendJwtError {
    BackendJwtError::DecodingError("Invalid device token profile".into())
}

fn valid_times(iat: i64, nbf: i64, exp: i64, max_lifetime: i64, now: i64) -> bool {
    iat > 0
        && iat <= now + 5
        && nbf == iat
        && exp > now
        && exp > iat
        && exp.saturating_sub(iat) <= max_lifetime
}

pub fn verify_enrollment(token: &str) -> Result<EnrollmentClaims, BackendJwtError> {
    let claims: EnrollmentClaims =
        backend_jwt::verify_typed(token, TokenType::DeviceEnrollment, ENROLLMENT_JOSE_TYPE)?;
    if claims.typ != TokenType::DeviceEnrollment
        || claims.scope != "device:enroll"
        || claims.sub.is_empty()
        || claims.enrollment_id.is_empty()
        || claims.device_id.is_empty()
        || claims.jti.is_empty()
        || claims.cnf.jkt.is_empty()
        || !valid_times(
            claims.iat,
            claims.nbf,
            claims.exp,
            86_400,
            chrono::Utc::now().timestamp(),
        )
    {
        return Err(invalid());
    }
    Ok(claims)
}

pub fn verify_session(token: &str) -> Result<SessionClaims, BackendJwtError> {
    let claims: SessionClaims =
        backend_jwt::verify_typed(token, TokenType::DeviceSession, SESSION_JOSE_TYPE)?;
    if claims.typ != TokenType::DeviceSession
        || claims.scope != "device:read device:presence"
        || claims.sub != claims.device_id
        || claims.device_id.is_empty()
        || claims.owner_id.is_empty()
        || claims.auth_epoch == 0
        || claims.jti.is_empty()
        || claims.cnf.jkt.is_empty()
        || !valid_times(
            claims.iat,
            claims.nbf,
            claims.exp,
            SESSION_SECONDS,
            chrono::Utc::now().timestamp(),
        )
    {
        return Err(invalid());
    }
    Ok(claims)
}

/// This untrusted inspection can only deny ordinary-user authentication. Device
/// routes still verify signatures, profiles, current registry state and DPoP.
pub fn is_device_credential(token: &str) -> bool {
    let token = token.strip_prefix("DPoP ").unwrap_or(token);
    if token.len() > 16_384 {
        return false;
    }
    if jsonwebtoken::decode_header(token)
        .ok()
        .and_then(|header| header.typ)
        .is_some_and(|typ| typ.starts_with("flow-like-device-") || typ.starts_with("flow-like-instance-") || typ == "dpop+jwt")
    {
        return true;
    }
    let Some(payload) = token.split('.').nth(1) else {
        return false;
    };
    let Some(payload) = URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return false;
    };
    payload
        .get("typ")
        .and_then(|value| value.as_str())
        .is_some_and(|typ| typ.starts_with("device_") || typ.starts_with("instance_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionClaims {
        let now = chrono::Utc::now().timestamp();
        SessionClaims {
            sub: "device".into(),
            device_id: "device".into(),
            owner_id: "owner".into(),
            auth_epoch: 1,
            cnf: Confirmation { jkt: "key".into() },
            scope: "device:read device:presence".into(),
            typ: TokenType::DeviceSession,
            iss: backend_jwt::issuer().into(),
            aud: TokenType::DeviceSession.audience().into(),
            iat: now,
            nbf: now,
            exp: now + 600,
            jti: "id".into(),
        }
    }

    #[test]
    fn exact_session_profile_and_type_are_required() {
        backend_jwt::init_for_tests();
        let mut claims = session();
        let token = backend_jwt::sign_typed(&claims, SESSION_JOSE_TYPE).unwrap();
        assert_eq!(verify_session(&token).unwrap().device_id, "device");
        assert!(is_device_credential(&token));
        assert!(verify_enrollment(&token).is_err());
        assert!(verify_session(&backend_jwt::sign(&claims).unwrap()).is_err());
        claims.typ = TokenType::Executor;
        assert!(
            verify_session(&backend_jwt::sign_typed(&claims, SESSION_JOSE_TYPE).unwrap()).is_err()
        );
        claims = session();
        claims.exp += 1;
        assert!(
            verify_session(&backend_jwt::sign_typed(&claims, SESSION_JOSE_TYPE).unwrap()).is_err()
        );
        claims = session();
        claims.auth_epoch = 0;
        assert!(
            verify_session(&backend_jwt::sign_typed(&claims, SESSION_JOSE_TYPE).unwrap()).is_err()
        );
    }

    #[test]
    fn device_payload_cannot_fall_back_to_a_human_token() {
        let token = format!(
            "{}.{}.invalid",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"ES256","typ":"JWT"}"#),
            URL_SAFE_NO_PAD.encode(br#"{"typ":"device_session","sub":"owner"}"#)
        );
        assert!(is_device_credential(&token));
        assert!(!is_device_credential("ordinary-opaque-token"));
    }

    #[test]
    fn signed_sessions_reject_changed_authority_scope_and_unknown_claims() {
        backend_jwt::init_for_tests();
        let now = chrono::Utc::now().timestamp();
        for (field, value) in [
            ("sub", serde_json::json!("owner")),
            ("device_id", serde_json::json!("")),
            ("owner_id", serde_json::json!("")),
            ("scope", serde_json::json!("device:*")),
            (
                "aud",
                serde_json::json!(TokenType::DeviceEnrollment.audience()),
            ),
            ("iss", serde_json::json!("https://other-issuer.example")),
            ("exp", serde_json::json!(now - 1)),
            ("nbf", serde_json::json!(now + 60)),
            ("admin", serde_json::json!(true)),
            ("cnf", serde_json::json!({"jkt":"key", "extra":"ignored?"})),
        ] {
            let mut claims = serde_json::to_value(session()).unwrap();
            claims[field] = value;
            let token = backend_jwt::sign_typed(&claims, SESSION_JOSE_TYPE).unwrap();
            assert!(verify_session(&token).is_err(), "accepted changed {field}");
        }
    }

    #[test]
    fn enrollment_profile_is_scoped_and_cannot_be_used_as_a_session() {
        backend_jwt::init_for_tests();
        let now = chrono::Utc::now().timestamp();
        let mut claims = EnrollmentClaims {
            sub: "owner".into(),
            enrollment_id: "enrollment".into(),
            device_id: "device".into(),
            cnf: Confirmation {
                jkt: "bootstrap".into(),
            },
            scope: "device:enroll".into(),
            typ: TokenType::DeviceEnrollment,
            iss: backend_jwt::issuer().into(),
            aud: TokenType::DeviceEnrollment.audience().into(),
            iat: now,
            nbf: now,
            exp: now + 86_400,
            jti: "enrollment-token-id".into(),
        };
        let token = backend_jwt::sign_typed(&claims, ENROLLMENT_JOSE_TYPE).unwrap();
        assert_eq!(verify_enrollment(&token).unwrap().sub, "owner");
        assert!(verify_session(&token).is_err());
        assert!(
            verify_enrollment(&backend_jwt::sign_typed(&claims, SESSION_JOSE_TYPE).unwrap())
                .is_err()
        );
        claims.exp += 1;
        assert!(
            verify_enrollment(&backend_jwt::sign_typed(&claims, ENROLLMENT_JOSE_TYPE).unwrap())
                .is_err()
        );
        claims.exp -= 1;
        claims.scope = "device:read device:presence".into();
        assert!(
            verify_enrollment(&backend_jwt::sign_typed(&claims, ENROLLMENT_JOSE_TYPE).unwrap())
                .is_err()
        );
    }
}
