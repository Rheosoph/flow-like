use crate::{
    backend_jwt::{self, BackendJwtError, TokenType},
    devices::jwt::Confirmation,
};
use serde::{Deserialize, Serialize};

pub(crate) const JOSE_TYPE: &str = "flow-like-instance-resource+jwt";
pub(crate) const TOKEN_SECONDS: i64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Actor {
    pub sub: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceClaims {
    pub sub: String,
    pub act: Actor,
    pub instance_id: String,
    pub device_id: String,
    pub device_auth_epoch: u64,
    pub key_epoch: u64,
    pub grant_id: String,
    pub authz_version: u64,
    pub billing_grant_id: String,
    pub billing_authz_version: u64,
    pub deployment_id: String,
    pub placement_id: String,
    pub project_id: String,
    pub app_id: Option<String>,
    pub cnf: Confirmation,
    pub dpop_nonce: String,
    pub scope: String,
    pub typ: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

pub(crate) fn verify(token: &str) -> Result<ResourceClaims, BackendJwtError> {
    let claims: ResourceClaims =
        backend_jwt::verify_typed(token, TokenType::InstanceResource, JOSE_TYPE)?;
    let now = chrono::Utc::now().timestamp();
    if claims.typ != TokenType::InstanceResource
        || claims.scope != "models:invoke"
        || claims.sub.is_empty()
        || claims.instance_id.is_empty()
        || claims.act.sub != format!("instance:{}", claims.instance_id)
        || claims.device_auth_epoch == 0
        || claims.key_epoch == 0
        || claims.authz_version == 0
        || claims.billing_authz_version == 0
        || claims.cnf.jkt.is_empty()
        || claims.dpop_nonce.len() != 43
        || claims.jti.is_empty()
        || claims.iat <= 0
        || claims.iat > now + 5
        || claims.nbf != claims.iat
        || claims.exp <= now
        || claims.exp <= claims.iat
        || claims.exp.saturating_sub(claims.iat) > TOKEN_SECONDS
    {
        return Err(BackendJwtError::DecodingError(
            "Invalid instance resource token profile".into(),
        ));
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_profiles_are_reserved_and_have_no_user_fallback() {
        backend_jwt::init_for_tests();
        let now = chrono::Utc::now().timestamp();
        let claims = ResourceClaims {
            sub: "delegate".into(),
            act: Actor {
                sub: "instance:workload".into(),
            },
            instance_id: "workload".into(),
            device_id: "device".into(),
            device_auth_epoch: 1,
            key_epoch: 1,
            grant_id: "grant".into(),
            authz_version: 1,
            billing_grant_id: "billing".into(),
            billing_authz_version: 1,
            deployment_id: "deployment".into(),
            placement_id: "placement".into(),
            project_id: "project".into(),
            app_id: None,
            cnf: Confirmation { jkt: "key".into() },
            dpop_nonce: "a".repeat(43),
            scope: "models:invoke".into(),
            typ: TokenType::InstanceResource,
            iss: backend_jwt::issuer().into(),
            aud: TokenType::InstanceResource.audience().into(),
            iat: now,
            nbf: now,
            exp: now + TOKEN_SECONDS,
            jti: "token-id".into(),
        };
        let token = backend_jwt::sign_typed(&claims, JOSE_TYPE).unwrap();
        assert!(verify(&token).is_ok());
        assert!(crate::devices::jwt::is_device_credential(&token));
        assert!(crate::devices::jwt::verify_session(&token).is_err());
        assert!(verify(&backend_jwt::sign(&claims).unwrap()).is_err());
        for (field, value) in [
            ("act", serde_json::json!({"sub":"delegate"})),
            ("scope", serde_json::json!("device:read")),
            (
                "aud",
                serde_json::json!(TokenType::DeviceSession.audience()),
            ),
            ("exp", serde_json::json!(now + TOKEN_SECONDS + 1)),
            ("extra", serde_json::json!(true)),
        ] {
            let mut changed = serde_json::to_value(&claims).unwrap();
            changed[field] = value;
            assert!(verify(&backend_jwt::sign_typed(&changed, JOSE_TYPE).unwrap()).is_err());
        }
    }
}
