use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerClaims {
    pub sub: String,
    pub job_id: String,
    pub package_id: String,
    pub version: String,
    pub callback_url: String,
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Debug, Clone)]
pub struct CompilerJwtParams {
    pub sub: String,
    pub job_id: String,
    pub package_id: String,
    pub version: String,
    pub callback_url: String,
    pub ttl_seconds: Option<i64>,
}

pub fn sign(params: CompilerJwtParams) -> Result<String, BackendJwtError> {
    let time = make_time_claims(TokenType::Compiler, params.ttl_seconds);

    let claims = CompilerClaims {
        sub: params.sub,
        job_id: params.job_id,
        package_id: params.package_id,
        version: params.version,
        callback_url: params.callback_url,
        token_type: TokenType::Compiler,
        iss: issuer().to_string(),
        aud: TokenType::Compiler.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    };

    backend_jwt::sign(&claims)
}

pub fn verify(token: &str) -> Result<CompilerClaims, BackendJwtError> {
    backend_jwt::verify::<CompilerClaims>(token, TokenType::Compiler)
}
