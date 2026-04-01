//! Interaction JWT module for human-in-the-loop authentication
//!
//! Only **Responder JWTs** are needed: the creation endpoint uses SSE streaming
//! so no creator JWT is required — the connection stays open.

use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use serde::{Deserialize, Serialize};

pub type InteractionJwtError = BackendJwtError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionClaims {
    pub sub: String,
    pub interaction_id: String,
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

pub struct InteractionJwtParams {
    pub sub: String,
    pub interaction_id: String,
    pub ttl_seconds: Option<i64>,
}

pub fn sign_responder(params: InteractionJwtParams) -> Result<String, InteractionJwtError> {
    let token_type = TokenType::InteractionResponder;
    let time = make_time_claims(token_type, params.ttl_seconds);

    let claims = InteractionClaims {
        sub: params.sub,
        interaction_id: params.interaction_id,
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

pub fn verify_responder(token: &str) -> Result<InteractionClaims, InteractionJwtError> {
    let claims: InteractionClaims = backend_jwt::verify(token, TokenType::InteractionResponder)?;
    if claims.token_type != TokenType::InteractionResponder {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: TokenType::InteractionResponder,
            got: claims.token_type,
        });
    }
    Ok(claims)
}
