//! Request-time authorization for services whose lifetime exceeds a credential lease.

use std::{fmt, future::Future, pin::Pin, time::SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceAudience {
    HostedModels,
    ProjectApi,
}

/// The provider binds the caller to its instance, project and grant. A request
/// supplies the actual HTTP target so possession proofs cover the dispatched call.
pub struct AuthorizationRequest<'a> {
    pub audience: ResourceAudience,
    pub method: &'a str,
    pub url: &'a str,
}

pub type AuthorizationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RequestAuthorization, AuthorizationError>> + Send + 'a>>;

pub trait RequestAuthorizer: Send + Sync {
    fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a>;

    /// An exact resource base supplied by the trusted credential broker. Clients
    /// must preserve its path instead of adding an assumed API version prefix.
    fn resource_base_url(&self, _audience: ResourceAudience) -> Option<String> {
        None
    }
}

/// Credentials are deliberately neither serializable nor visible in debug output.
pub struct RequestAuthorization {
    authorization: String,
    dpop: Option<String>,
    expires_at: SystemTime,
}

impl RequestAuthorization {
    pub fn new(
        authorization: String,
        dpop: Option<String>,
        expires_at: SystemTime,
    ) -> Result<Self, AuthorizationError> {
        let result = Self {
            authorization,
            dpop,
            expires_at,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn authorization(&self) -> &str {
        &self.authorization
    }

    pub fn dpop(&self) -> Option<&str> {
        self.dpop.as_deref()
    }

    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    pub fn validate(&self) -> Result<(), AuthorizationError> {
        if self.expires_at <= SystemTime::now() {
            return Err(AuthorizationError::Expired);
        }
        let (scheme, token) = self
            .authorization
            .split_once(' ')
            .ok_or(AuthorizationError::InvalidResponse)?;
        let valid_value = |value: &str| {
            !value.is_empty()
                && value.len() <= 16 * 1024
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        };
        if !valid_value(token)
            || !matches!(
                (scheme, self.dpop.as_ref()),
                ("Bearer", None) | ("DPoP", Some(_))
            )
            || self
                .dpop
                .as_deref()
                .is_some_and(|proof| !valid_value(proof))
        {
            return Err(AuthorizationError::InvalidResponse);
        }
        Ok(())
    }
}

impl fmt::Debug for RequestAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestAuthorization")
            .field("authorization", &"[redacted]")
            .field("has_dpop", &self.dpop.is_some())
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Fixed error messages prevent issuer responses from leaking credentials into logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationError {
    Denied,
    Unavailable,
    Expired,
    InvalidRequest,
    InvalidResponse,
}

impl fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Denied => "Resource authorization was denied",
            Self::Unavailable => "Resource authorization is temporarily unavailable",
            Self::Expired => "Resource authorization has expired",
            Self::InvalidRequest => "Request is outside the authorized resource endpoint",
            Self::InvalidResponse => "Resource authorizer returned invalid credentials",
        })
    }
}

impl std::error::Error for AuthorizationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rejects_expired_or_malformed_authorization_without_exposing_secrets() {
        let expiry = SystemTime::now() + Duration::from_secs(60);
        assert!(
            RequestAuthorization::new("Bearer secret\r\nInjected: value".into(), None, expiry)
                .is_err()
        );
        assert!(RequestAuthorization::new("DPoP token".into(), None, expiry).is_err());
        assert!(
            RequestAuthorization::new("Bearer token".into(), None, SystemTime::UNIX_EPOCH).is_err()
        );
        let authorization = RequestAuthorization::new(
            "DPoP private-token".into(),
            Some("private-proof".into()),
            expiry,
        )
        .unwrap();
        let debug = format!("{authorization:?}");
        assert!(!debug.contains("private-token"));
        assert!(!debug.contains("private-proof"));
    }
}
