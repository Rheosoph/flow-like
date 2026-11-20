//! The token that proves a query request comes from this API, bound to one request body.

use flow_like_query::{AUDIENCE, JOSE_TYPE, QueryClaims, RequestSigner};

use crate::backend_jwt;

/// Long enough for clock skew and a cold start, far below the service's 300 s ceiling.
pub(super) const TOKEN_LIFETIME_SECONDS: i64 = 60;

pub(super) struct BackendSigner {
    subject: String,
}

impl BackendSigner {
    pub(super) fn new(subject: &str) -> Self {
        Self {
            subject: subject.to_string(),
        }
    }
}

pub(super) fn claims(subject: &str, request_id: &str, body_sha256: &str, now: i64) -> QueryClaims {
    QueryClaims {
        iss: backend_jwt::issuer().to_string(),
        sub: subject.to_string(),
        aud: AUDIENCE.to_string(),
        iat: now,
        nbf: now,
        exp: now + TOKEN_LIFETIME_SECONDS,
        jti: uuid::Uuid::new_v4().to_string(),
        request_id: request_id.to_string(),
        body_sha256: body_sha256.to_string(),
    }
}

impl RequestSigner for BackendSigner {
    fn sign(&self, request_id: &str, body_sha256: &str) -> Result<String, String> {
        let claims = claims(
            &self.subject,
            request_id,
            body_sha256,
            chrono::Utc::now().timestamp(),
        );
        backend_jwt::sign_typed(&claims, JOSE_TYPE).map_err(|error| error.to_string())
    }
}
