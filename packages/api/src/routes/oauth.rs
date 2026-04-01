use std::collections::HashMap;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::state::AppState;

const OAUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// OAuth configs loaded at build time (without secrets)
static OAUTH_CONFIG: &str = include_str!(concat!(env!("OUT_DIR"), "/oauth_config.json"));

/// How the provider expects credentials on token requests
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum AuthMethod {
    /// POST form-encoded with client_id (+ optional client_secret) in body
    #[default]
    FormPost,
    /// HTTP Basic header with JSON body (e.g. Notion)
    BasicJson,
}

impl AuthMethod {
    fn from_str_opt(s: Option<&str>) -> Self {
        match s {
            Some("basic_json") => Self::BasicJson,
            _ => Self::FormPost,
        }
    }
}

/// Config as stored in flow-like.config.json (without resolved secrets)
#[derive(Debug, Clone, Deserialize)]
struct OAuthProviderConfig {
    #[serde(default)]
    client_id: Option<String>,
    /// Environment variable name containing the client secret
    client_secret_env: Option<String>,
    token_url: String,
    revoke_url: Option<String>,
    userinfo_url: Option<String>,
    device_auth_url: Option<String>,
    auth_method: Option<String>,
}

/// Resolved config with secrets loaded from env at runtime
#[derive(Debug, Clone)]
struct ResolvedOAuthConfig {
    client_id: Option<String>,
    client_secret: Option<String>,
    token_url: String,
    revoke_url: Option<String>,
    userinfo_url: Option<String>,
    device_auth_url: Option<String>,
    auth_method: AuthMethod,
}

async fn get_oauth_configs(secrets: &SecretStore) -> HashMap<String, ResolvedOAuthConfig> {
    let raw_configs: HashMap<String, OAuthProviderConfig> =
        flow_like_types::json::from_str(OAUTH_CONFIG).unwrap_or_default();

    let mut resolved = HashMap::with_capacity(raw_configs.len());
    for (provider_id, cfg) in raw_configs {
        let client_secret = if let Some(env_name) = &cfg.client_secret_env {
            secrets
                .get_secret_string(&SecretRef::new(env_name.as_str()))
                .await
                .ok()
                .map(|s| s.expose_secret().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        };

        let auth_method = AuthMethod::from_str_opt(cfg.auth_method.as_deref());

        resolved.insert(
            provider_id,
            ResolvedOAuthConfig {
                client_id: cfg.client_id,
                client_secret,
                token_url: cfg.token_url,
                revoke_url: cfg.revoke_url,
                userinfo_url: cfg.userinfo_url,
                device_auth_url: cfg.device_auth_url,
                auth_method,
            },
        );
    }
    resolved
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/token/{provider_id}", post(token_exchange))
        .route("/refresh/{provider_id}", post(token_refresh))
        .route("/device/start/{provider_id}", post(device_start))
        .route("/device/poll/{provider_id}", post(device_poll))
        .route("/userinfo/{provider_id}", post(userinfo))
        .route("/revoke/{provider_id}", post(revoke_token))
}

/// Custom error type for OAuth proxy errors
pub struct OAuthProxyError {
    status: StatusCode,
    message: String,
}

impl OAuthProxyError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for OAuthProxyError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": "oauth_proxy_error",
            "error_description": self.message,
        });
        (self.status, Json(body)).into_response()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenExchangeRequest {
    /// The authorization code from the OAuth flow
    pub code: String,
    /// The redirect URI used in the authorization request
    pub redirect_uri: String,
    /// The PKCE code verifier (if PKCE was used)
    pub code_verifier: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRefreshRequest {
    /// The refresh token
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeviceStartRequest {
    /// OAuth scopes for device flow
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DevicePollRequest {
    /// Device code from the provider's device authorization endpoint
    pub device_code: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UserInfoRequest {
    /// OAuth access token
    pub access_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RevokeTokenRequest {
    /// OAuth access token to revoke
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeviceStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    #[serde(default = "default_poll_interval")]
    pub interval: i64,
}

fn default_poll_interval() -> i64 {
    5
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

fn parse_form_values(response_text: &str) -> HashMap<String, String> {
    response_text
        .split('&')
        .filter_map(|pair| {
            if pair.is_empty() {
                return None;
            }
            let mut parts = pair.splitn(2, '=');
            let key = parts.next()?.to_string();
            let value = parts.next().unwrap_or("").to_string();
            Some((key, value))
        })
        .collect()
}

fn parse_json_or_form_value(response_text: &str) -> serde_json::Value {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(response_text) {
        return value;
    }

    let form = parse_form_values(response_text);
    if form.is_empty() {
        serde_json::json!({ "raw": response_text })
    } else {
        serde_json::json!(form)
    }
}

fn provider_error_message(response_text: &str) -> String {
    if let Ok(error_resp) = serde_json::from_str::<ErrorResponse>(response_text) {
        return format!(
            "{} - {}",
            error_resp.error,
            error_resp.error_description.unwrap_or_default()
        );
    }

    let form = parse_form_values(response_text);
    if let Some(error) = form.get("error") {
        let description = form.get("error_description").cloned().unwrap_or_default();
        return format!("{} - {}", error, description);
    }

    response_text.to_string()
}

fn parse_token_response(response_text: &str) -> Result<TokenResponse, OAuthProxyError> {
    if let Ok(token_response) = serde_json::from_str::<TokenResponse>(response_text) {
        if token_response.access_token.is_empty() {
            return Err(OAuthProxyError::new(
                StatusCode::BAD_GATEWAY,
                "Token response did not include access_token",
            ));
        }
        return Ok(token_response);
    }

    let form = parse_form_values(response_text);
    let token_response = TokenResponse {
        access_token: form.get("access_token").cloned().unwrap_or_default(),
        refresh_token: form.get("refresh_token").cloned(),
        expires_in: form
            .get("expires_in")
            .and_then(|value| value.parse::<i64>().ok()),
        token_type: form.get("token_type").cloned(),
        id_token: form.get("id_token").cloned(),
        scope: form.get("scope").cloned(),
        workspace_id: form.get("workspace_id").cloned(),
        workspace_name: form.get("workspace_name").cloned(),
        workspace_icon: form.get("workspace_icon").cloned(),
        bot_id: form.get("bot_id").cloned(),
    };

    if token_response.access_token.is_empty() {
        let message = if form.is_empty() {
            format!("Failed to parse token response: {}", response_text)
        } else {
            provider_error_message(response_text)
        };
        return Err(OAuthProxyError::new(StatusCode::BAD_GATEWAY, message));
    }

    Ok(token_response)
}

fn require_provider_config<'a>(
    provider_id: &str,
    configs: &'a HashMap<String, ResolvedOAuthConfig>,
) -> Result<&'a ResolvedOAuthConfig, OAuthProxyError> {
    configs.get(provider_id).ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::NOT_FOUND,
            format!("OAuth provider '{}' not found in config", provider_id),
        )
    })
}

fn require_client_id<'a>(
    provider_id: &str,
    provider_config: &'a ResolvedOAuthConfig,
) -> Result<&'a str, OAuthProxyError> {
    provider_config.client_id.as_deref().ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "Client ID not configured for provider '{}' in flow-like.config.json",
                provider_id
            ),
        )
    })
}

fn require_client_secret<'a>(
    provider_id: &str,
    provider_config: &'a ResolvedOAuthConfig,
) -> Result<&'a str, OAuthProxyError> {
    provider_config.client_secret.as_deref().ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "Client secret not configured for provider '{}'. Set the environment variable specified in client_secret_env.",
                provider_id
            ),
        )
    })
}

fn build_oauth_client() -> Result<flow_like_types::reqwest::Client, OAuthProxyError> {
    flow_like_types::reqwest::Client::builder()
        .timeout(OAUTH_REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| OAuthProxyError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[utoipa::path(
    post,
    path = "/oauth/token/{provider_id}",
    tag = "oauth",
    params(
        ("provider_id" = String, Path, description = "OAuth provider identifier")
    ),
    request_body = TokenExchangeRequest,
    responses(
        (status = 200, description = "Token exchange successful", body = TokenResponse),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 404, description = "Provider not found", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
#[tracing::instrument(name = "POST /oauth/token/:provider_id", skip(state))]
pub async fn token_exchange(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<TokenExchangeRequest>,
) -> Result<Json<TokenResponse>, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let client_id = require_client_id(&provider_id, provider_config)?;

    let client = build_oauth_client()?;

    let response = match provider_config.auth_method {
        AuthMethod::BasicJson => {
            let client_secret = require_client_secret(&provider_id, provider_config)?;
            let credentials = flow_like_types::base64::Engine::encode(
                &flow_like_types::base64::engine::general_purpose::STANDARD,
                format!("{}:{}", client_id, client_secret),
            );

            let mut json_payload = serde_json::json!({
                "grant_type": "authorization_code",
                "code": request.code,
                "redirect_uri": request.redirect_uri,
            });
            if let Some(code_verifier) = request.code_verifier {
                json_payload["code_verifier"] = serde_json::Value::String(code_verifier);
            }

            client
                .post(&provider_config.token_url)
                .header("Authorization", format!("Basic {}", credentials))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&json_payload)
                .send()
                .await
                .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?
        }
        AuthMethod::FormPost => {
            let mut params = vec![
                ("grant_type", "authorization_code".to_string()),
                ("code", request.code),
                ("redirect_uri", request.redirect_uri),
                ("client_id", client_id.to_string()),
            ];

            if let Some(code_verifier) = request.code_verifier {
                params.push(("code_verifier", code_verifier));
            }
            if let Some(client_secret) = provider_config.client_secret.as_ref() {
                params.push(("client_secret", client_secret.clone()));
            }

            client
                .post(&provider_config.token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .form(&params)
                .send()
                .await
                .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?
        }
    };

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !status.is_success() {
        let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(OAuthProxyError::new(
            status_code,
            format!(
                "Token exchange failed: {}",
                provider_error_message(&response_text)
            ),
        ));
    }

    let token_response = parse_token_response(&response_text)?;
    Ok(Json(token_response))
}

#[utoipa::path(
    post,
    path = "/oauth/refresh/{provider_id}",
    tag = "oauth",
    params(
        ("provider_id" = String, Path, description = "OAuth provider identifier")
    ),
    request_body = TokenRefreshRequest,
    responses(
        (status = 200, description = "Token refresh successful", body = TokenResponse),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 404, description = "Provider not found", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
#[tracing::instrument(name = "POST /oauth/refresh/:provider_id", skip(state))]
pub async fn token_refresh(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<TokenRefreshRequest>,
) -> Result<Json<TokenResponse>, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let client_id = require_client_id(&provider_id, provider_config)?;

    let client = build_oauth_client()?;

    let response = match provider_config.auth_method {
        AuthMethod::BasicJson => {
            let client_secret = require_client_secret(&provider_id, provider_config)?;
            let credentials = flow_like_types::base64::Engine::encode(
                &flow_like_types::base64::engine::general_purpose::STANDARD,
                format!("{}:{}", client_id, client_secret),
            );

            client
                .post(&provider_config.token_url)
                .header("Authorization", format!("Basic {}", credentials))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&serde_json::json!({
                    "grant_type": "refresh_token",
                    "refresh_token": request.refresh_token,
                }))
                .send()
                .await
                .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?
        }
        AuthMethod::FormPost => {
            let mut params = vec![
                ("grant_type", "refresh_token".to_string()),
                ("refresh_token", request.refresh_token),
                ("client_id", client_id.to_string()),
            ];
            if let Some(client_secret) = provider_config.client_secret.as_ref() {
                params.push(("client_secret", client_secret.clone()));
            }

            client
                .post(&provider_config.token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .form(&params)
                .send()
                .await
                .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?
        }
    };

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !status.is_success() {
        let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(OAuthProxyError::new(
            status_code,
            format!(
                "Token refresh failed: {}",
                provider_error_message(&response_text)
            ),
        ));
    }

    let token_response = parse_token_response(&response_text)?;
    Ok(Json(token_response))
}

#[tracing::instrument(name = "POST /oauth/device/start/:provider_id", skip(state))]
pub async fn device_start(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<DeviceStartRequest>,
) -> Result<Json<DeviceStartResponse>, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let device_auth_url = provider_config.device_auth_url.as_ref().ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not support device flow", provider_id),
        )
    })?;
    let client_id = require_client_id(&provider_id, provider_config)?;

    let mut params = vec![("client_id", client_id.to_string())];
    if let Some(scope) = request.scope
        && !scope.trim().is_empty()
    {
        params.push(("scope", scope));
    }
    if let Some(client_secret) = provider_config.client_secret.as_ref() {
        params.push(("client_secret", client_secret.clone()));
    }

    let client = build_oauth_client()?;
    let response = client
        .post(device_auth_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !status.is_success() {
        let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(OAuthProxyError::new(
            status_code,
            format!(
                "Device authorization failed: {}",
                provider_error_message(&response_text)
            ),
        ));
    }

    let mut payload = serde_json::from_str::<DeviceStartResponse>(&response_text).map_err(|e| {
        OAuthProxyError::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "Failed to parse device authorization response: {} - {}",
                e, response_text
            ),
        )
    })?;
    if payload.interval <= 0 {
        payload.interval = default_poll_interval();
    }

    Ok(Json(payload))
}

#[tracing::instrument(name = "POST /oauth/device/poll/:provider_id", skip(state))]
pub async fn device_poll(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<DevicePollRequest>,
) -> Result<Response, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let client_id = require_client_id(&provider_id, provider_config)?;

    let mut params = vec![
        ("client_id", client_id.to_string()),
        ("device_code", request.device_code),
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:device_code".to_string(),
        ),
    ];
    if let Some(client_secret) = provider_config.client_secret.as_ref() {
        params.push(("client_secret", client_secret.clone()));
    }

    let client = build_oauth_client()?;
    let response = client
        .post(&provider_config.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let payload = parse_json_or_form_value(&response_text);

    Ok((status, Json(payload)).into_response())
}

#[tracing::instrument(name = "POST /oauth/userinfo/:provider_id", skip(state))]
pub async fn userinfo(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<UserInfoRequest>,
) -> Result<Response, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let userinfo_url = provider_config.userinfo_url.as_ref().ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Provider '{}' does not expose userinfo endpoint",
                provider_id
            ),
        )
    })?;

    let client = build_oauth_client()?;
    let response = client
        .get(userinfo_url)
        .header("Accept", "application/json")
        .bearer_auth(request.access_token)
        .send()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !status.is_success() {
        let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(OAuthProxyError::new(
            status_code,
            format!(
                "Userinfo request failed: {}",
                provider_error_message(&response_text)
            ),
        ));
    }

    let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    let payload = parse_json_or_form_value(&response_text);
    Ok((status_code, Json(payload)).into_response())
}

#[tracing::instrument(name = "POST /oauth/revoke/:provider_id", skip(state))]
pub async fn revoke_token(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<RevokeTokenRequest>,
) -> Result<StatusCode, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let revoke_url = provider_config.revoke_url.as_ref().ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not expose revoke endpoint", provider_id),
        )
    })?;
    let client_id = require_client_id(&provider_id, provider_config)?;

    let mut params = vec![
        ("token", request.token),
        ("client_id", client_id.to_string()),
    ];
    if let Some(client_secret) = provider_config.client_secret.as_ref() {
        params.push(("client_secret", client_secret.clone()));
    }

    let client = build_oauth_client()?;
    let response = client
        .post(revoke_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !status.is_success() {
        let status_code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        return Err(OAuthProxyError::new(
            status_code,
            format!(
                "Token revoke failed: {}",
                provider_error_message(&response_text)
            ),
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}
