//! Channel responder JWT: the capability a client presents to `POST /channels/{cid}/push` and
//! `GET /channels/{cid}/grant`. Bound to one channel and one subject; the `transport` claim
//! tells the push endpoint whether the waiter polls a row or listens on a cloud transport.

use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use serde::{Deserialize, Serialize};

pub type ChannelJwtError = BackendJwtError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelClaims {
    pub sub: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Wire tag from `flow_like_types::channel::CHANNEL_TRANSPORT_*`.
    pub transport: String,
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

pub struct ChannelJwtParams {
    pub sub: String,
    pub channel_id: String,
    pub app_id: Option<String>,
    pub transport: String,
    pub ttl_seconds: Option<i64>,
}

pub fn sign_channel_responder(params: ChannelJwtParams) -> Result<String, ChannelJwtError> {
    let token_type = TokenType::ChannelResponder;
    let time = make_time_claims(token_type, params.ttl_seconds);

    let claims = ChannelClaims {
        sub: params.sub,
        channel_id: params.channel_id,
        app_id: params.app_id,
        transport: params.transport,
        token_type,
        iss: issuer().to_string(),
        aud: token_type.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };

    backend_jwt::sign(&claims)
}

pub fn verify_channel_responder(token: &str) -> Result<ChannelClaims, ChannelJwtError> {
    let claims: ChannelClaims = backend_jwt::verify(token, TokenType::ChannelResponder)?;
    if claims.token_type != TokenType::ChannelResponder {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: TokenType::ChannelResponder,
            got: claims.token_type,
        });
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::{CHANNEL_TRANSPORT_AWS_MQTT, CHANNEL_TRANSPORT_HTTP};

    fn params(transport: &str) -> ChannelJwtParams {
        ChannelJwtParams {
            sub: "user-1".into(),
            channel_id: "run-1".into(),
            app_id: Some("app-1".into()),
            transport: transport.into(),
            ttl_seconds: Some(120),
        }
    }

    #[test]
    fn roundtrip_keeps_channel_binding_and_transport() {
        backend_jwt::init_for_tests();
        let token = sign_channel_responder(params(CHANNEL_TRANSPORT_AWS_MQTT)).unwrap();
        let claims = verify_channel_responder(&token).unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.channel_id, "run-1");
        assert_eq!(claims.app_id.as_deref(), Some("app-1"));
        assert_eq!(claims.transport, CHANNEL_TRANSPORT_AWS_MQTT);
        assert_eq!(claims.token_type, TokenType::ChannelResponder);
        assert_eq!(claims.aud, "flow-like-channel-responder");
        assert_eq!(claims.exp - claims.iat, 120);
    }

    #[test]
    fn other_token_types_are_rejected() {
        backend_jwt::init_for_tests();
        let executor = crate::execution::sign_execution_jwt(crate::execution::ExecutionJwtParams {
            user_id: "user-1".into(),
            technical_user_id: None,
            run_id: "run-1".into(),
            app_id: "app-1".into(),
            board_id: "board-1".into(),
            event_id: None,
            app_chain: None,
            correlation: None,
            callback_url: "https://api.test".into(),
            token_type: TokenType::Executor,
            ttl_seconds: Some(60),
            shadow: None,
        })
        .unwrap();
        assert!(verify_channel_responder(&executor).is_err());

        let http = sign_channel_responder(params(CHANNEL_TRANSPORT_HTTP)).unwrap();
        assert!(crate::execution::verify_execution_jwt(&http).is_err());
    }
}
