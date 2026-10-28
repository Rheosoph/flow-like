use std::collections::HashMap;
use std::time::Duration;

use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use flow_like_secrets::{ExposeSecret, SecretRef, SecretStore};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::middleware::jwt::{AppUser, SHADOW_SIDE_EFFECT_REFUSED};
use crate::runtime_config::OAuthProviderConfig;
use crate::state::AppState;

mod connections;

// Every handler here skips its `request` argument: the bodies carry live
// credentials (authorization codes, refresh tokens, access tokens, device
// codes) and `#[tracing::instrument]` would `Debug`-format them into every
// subscriber, including the persisted telemetry span table. `provider_id` is
// the safe correlation field and stays recorded.
const OAUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg(test)]
mod runtime_config_tests {
    use super::*;
    use flow_like_secrets::{FileProviderConfig, ProviderConfig, SecretStoreConfig};

    #[test]
    fn form_tokens_decode_reserved_characters_and_spaces() {
        let token = parse_token_response(
            "access_token=a%2Bb%2Fc%3D&refresh_token=r%26x%3Dy&scope=read+write&expires_in=3600",
        )
        .unwrap_or_else(|_| panic!("valid form token"));
        assert_eq!(token.access_token, "a+b/c=");
        assert_eq!(token.refresh_token.as_deref(), Some("r&x=y"));
        assert_eq!(token.scope.as_deref(), Some("read write"));
        assert_eq!(token.expires_in, Some(3600));
    }

    #[test]
    fn provider_rejection_does_not_invalidate_the_hub_session() {
        assert_eq!(provider_status(401), StatusCode::BAD_REQUEST);
        assert_eq!(connections::reconnect().status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn replays_cannot_refresh_oauth_handles() {
        let executor = |shadow| {
            AppUser::Executor(crate::middleware::jwt::ExecutorUser {
                sub: "user-1".into(),
                app_id: "app-1".into(),
                run_id: "run-1".into(),
                technical_user_id: None,
                app_chain: None,
                correlation: None,
                shadow,
                hosted_frontend: false,
                event_id: None,
                custom_bits_owner: None,
                unattended: false,
            })
        };
        let refusal = refuse_shadow_refresh(&executor(true)).unwrap_err();
        assert_eq!(refusal.status, StatusCode::FORBIDDEN);
        assert_eq!(refusal.code, SHADOW_SIDE_EFFECT_REFUSED);
        assert_eq!(
            refusal.message,
            "Shadow and regression runs cannot refresh OAuth tokens"
        );
        assert!(refuse_shadow_refresh(&executor(false)).is_ok());

        let tokens = |refresh: serde_json::Value| {
            HashMap::from([(
                "provider".to_string(),
                serde_json::json!({"access_token": "access", "refresh_token": refresh}),
            )])
        };
        let handle = tokens(format!("{}connection", connections::HANDLE_PREFIX).into());
        let refusal = incoming_token_owner(&executor(true), &handle).unwrap_err();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        assert_eq!(refusal.public_code(), SHADOW_SIDE_EFFECT_REFUSED);
        assert_eq!(
            refusal.public_message(),
            Some("Shadow and regression runs cannot refresh OAuth tokens")
        );
        assert_eq!(
            incoming_token_owner(&executor(false), &handle).unwrap(),
            None
        );
        let access_only = tokens(serde_json::Value::Null);
        assert_eq!(
            incoming_token_owner(&executor(true), &access_only).unwrap(),
            None
        );
        let raw = tokens("provider-refresh-token".into());
        for shadow in [true, false] {
            let refusal = incoming_token_owner(&executor(shadow), &raw).unwrap_err();
            assert_eq!(
                refusal.status(),
                StatusCode::BAD_REQUEST,
                "shadow = {shadow}"
            );
        }
    }

    #[test]
    fn replays_are_refused_before_any_oauth_handle_resolves() {
        let source = include_str!("oauth.rs");
        let handler = |signature: &str| {
            let body = source
                .split_once(signature)
                .unwrap_or_else(|| panic!("{signature} exists"))
                .1;
            body.split_once("\n}\n").map_or(body, |(body, _)| body)
        };
        let position = |body: &str, step: &str| {
            body.find(step)
                .unwrap_or_else(|| panic!("the handler calls {step}"))
        };

        let incoming = handler("\npub(crate) async fn authorize_incoming_oauth_tokens(");
        assert!(
            position(incoming, "incoming_token_owner(user, tokens)?")
                < position(incoming, "resolve_oauth_token(")
        );
        let refresh = handler("\npub async fn token_refresh(");
        let refused = position(refresh, "refuse_shadow_refresh(&user)?");
        for step in ["resolve_connection_config(", ".adopt(", ".resolve_scoped("] {
            assert!(refused < position(refresh, step), "{step}");
        }
    }

    #[test]
    fn public_client_overrides_keep_the_trusted_endpoint_without_a_secret() {
        let config = ResolvedOAuthConfig {
            client_id: Some("default".into()),
            client_secret: Some("hub-secret".into()),
            requires_secret_proxy: false,
            token_url: "https://trusted.test/token".into(),
            revoke_url: None,
            userinfo_url: None,
            device_auth_url: None,
            auth_method: AuthMethod::FormPost,
        };
        let custom = select_client_config("provider", &config, Some("custom-client")).unwrap();
        assert_eq!(custom.client_id.as_deref(), Some("custom-client"));
        assert!(custom.client_secret.is_none());
        assert_eq!(custom.token_url, config.token_url);
        let confidential = ResolvedOAuthConfig {
            requires_secret_proxy: true,
            ..config
        };
        assert!(select_client_config("provider", &confidential, Some("custom-client")).is_err());
        assert_eq!(
            normalize_refresh_scope(Some("write read  read"))
                .unwrap()
                .as_deref(),
            Some("read write")
        );
    }

    #[tokio::test]
    async fn proxy_and_sink_provider_resolver_uses_runtime_config_and_keeps_secrets_private() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("private-reference-marker"),
            "private-secret-marker",
        )
        .unwrap();
        let secrets = SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(FileProviderConfig {
                    root_path: directory.path().into(),
                    trim_trailing_newline: false,
                })),
        )
        .unwrap();
        let mut document: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../apps/backend/kubernetes/flow-like.config.example.json"
        ))
        .unwrap();
        document["oauth_providers"] = serde_json::json!({"runtime-only": {
            "name":"Runtime provider", "client_id":"runtime-client", "client_secret_env":"private-reference-marker",
            "auth_url":"https://runtime-provider.example.test/auth", "token_url":"https://runtime-provider.example.test/token",
            "auth_method":"basic_json", "revoke_url":"https://runtime-provider.example.test/revoke"
        }});
        let effective =
            crate::runtime_config::EffectiveConfig::parse(&document.to_string()).unwrap();
        let providers = get_oauth_configs(&secrets, &effective.oauth_providers).await;
        assert_eq!(providers.len(), 1);
        let provider = &providers["runtime-only"];
        assert_eq!(provider.client_id.as_deref(), Some("runtime-client"));
        assert_eq!(
            provider.token_url,
            "https://runtime-provider.example.test/token"
        );
        assert_eq!(provider.auth_method, AuthMethod::BasicJson);
        assert!(provider.client_secret.as_deref() == Some("private-secret-marker"));
        assert!(
            effective.hub.oauth_providers["runtime-only"]
                .client_secret
                .is_none()
        );
        let public =
            crate::public_hub_value(serde_json::to_value(&effective.hub).unwrap()).to_string();
        assert!(!public.contains("private-secret-marker"));
        assert!(!public.contains("private-reference-marker"));
        assert!(
            get_oauth_configs(&secrets, &HashMap::new())
                .await
                .is_empty()
        );
    }
}

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

/// Resolved config with secrets loaded from env at runtime
#[derive(Debug, Clone)]
struct ResolvedOAuthConfig {
    client_id: Option<String>,
    client_secret: Option<String>,
    requires_secret_proxy: bool,
    token_url: String,
    revoke_url: Option<String>,
    userinfo_url: Option<String>,
    device_auth_url: Option<String>,
    auth_method: AuthMethod,
}

async fn get_oauth_configs(
    secrets: &SecretStore,
    raw_configs: &HashMap<String, OAuthProviderConfig>,
) -> HashMap<String, ResolvedOAuthConfig> {
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
            provider_id.clone(),
            ResolvedOAuthConfig {
                client_id: cfg.client_id.clone(),
                client_secret,
                requires_secret_proxy: cfg.requires_secret_proxy,
                token_url: cfg.token_url.clone(),
                revoke_url: cfg.revoke_url.clone(),
                userinfo_url: cfg.userinfo_url.clone(),
                device_auth_url: cfg.device_auth_url.clone(),
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
        .route("/connection/{provider_id}", post(adopt_connection))
        .route("/device/start/{provider_id}", post(device_start))
        .route("/device/poll/{provider_id}", post(device_poll))
        .route("/userinfo/{provider_id}", post(userinfo))
        .route("/revoke/{provider_id}", post(revoke_token))
}

/// Custom error type for OAuth proxy errors
#[derive(Debug)]
pub struct OAuthProxyError {
    code: &'static str,
    status: StatusCode,
    message: String,
}

impl OAuthProxyError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self::coded(status, "oauth_proxy_error", message)
    }

    fn coded(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for OAuthProxyError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": self.code,
            "error_description": self.message,
        });
        (self.status, Json(body)).into_response()
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenExchangeRequest {
    pub client_id: Option<String>,
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
    pub client_id: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeviceStartRequest {
    pub client_id: Option<String>,
    /// OAuth scopes for device flow
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DevicePollRequest {
    pub client_id: Option<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
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
            let key = urlencoding::decode(&parts.next()?.replace('+', " "))
                .ok()?
                .into_owned();
            let value = urlencoding::decode(&parts.next().unwrap_or("").replace('+', " "))
                .ok()?
                .into_owned();
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

fn provider_status(status: u16) -> StatusCode {
    // A provider rejection must not trigger Hub session renewal or sign-out.
    if status == 401 {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY)
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
    provider_config
        .client_id
        .as_deref()
        .filter(|client| !client.is_empty())
        .ok_or_else(|| {
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

const REFRESH_OAUTH_TOKENS: &str = "refresh OAuth tokens";

fn refuse_shadow_refresh(user: &AppUser) -> Result<(), OAuthProxyError> {
    user.deny_shadow_execution(REFRESH_OAUTH_TOKENS)
        .map_err(|refusal| {
            OAuthProxyError::coded(
                refusal.status(),
                SHADOW_SIDE_EFFECT_REFUSED,
                refusal.public_message().unwrap_or_default(),
            )
        })
}

fn require_owner(user: &AppUser) -> Result<String, OAuthProxyError> {
    user.sub().map_err(|_| {
        OAuthProxyError::new(
            if matches!(user, AppUser::Unauthorized) {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::FORBIDDEN
            },
            "Sign in to the hub with a user account to manage OAuth connections",
        )
    })
}

fn select_client_config(
    provider: &str,
    config: &ResolvedOAuthConfig,
    supplied: Option<&str>,
) -> Result<ResolvedOAuthConfig, OAuthProxyError> {
    let supplied = supplied.filter(|client| !client.is_empty());
    let mut selected = config.clone();
    if let Some(client) = supplied {
        if client.len() > 512 {
            return Err(OAuthProxyError::new(
                StatusCode::BAD_REQUEST,
                "OAuth client ID is too long",
            ));
        }
        if config.client_id.as_deref() != Some(client) {
            if config.requires_secret_proxy || config.auth_method != AuthMethod::FormPost {
                return Err(OAuthProxyError::new(
                    StatusCode::BAD_REQUEST,
                    "The OAuth client does not match this hub's confidential provider configuration",
                ));
            }
            selected.client_id = Some(client.to_owned());
            selected.client_secret = None;
        }
    }
    require_client_id(provider, &selected)?;
    if selected.requires_secret_proxy && selected.client_secret.is_none() {
        return Err(OAuthProxyError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "The OAuth provider client secret is unavailable",
        ));
    }
    Ok(selected)
}

async fn resolve_connection_config(
    state: &crate::state::State,
    provider: &str,
    config: &ResolvedOAuthConfig,
    refresh: &str,
    supplied_client: Option<&str>,
) -> Result<ResolvedOAuthConfig, OAuthProxyError> {
    if !refresh.starts_with(connections::HANDLE_PREFIX) {
        return select_client_config(provider, config, supplied_client);
    }
    let client = connections::Connections::client_id(&state.db, refresh).await?;
    if supplied_client.is_some_and(|supplied| supplied != client) {
        return Err(connections::reconnect());
    }
    // The token endpoint always comes from trusted Hub configuration. Reading the
    // connection below verifies its fingerprint against that endpoint and client.
    select_client_config(provider, config, Some(&client))
}

fn connection_store<'a>(
    state: &'a crate::state::State,
    provider_id: &str,
    config: &ResolvedOAuthConfig,
    owner: Option<&'a str>,
) -> Result<connections::Connections<'a>, OAuthProxyError> {
    Ok(connections::Connections::new(
        &state.db,
        &state.encryption_key,
        provider_id,
        require_client_id(provider_id, config)?,
        &config.token_url,
        owner,
    ))
}

async fn adopt_response(
    state: &crate::state::State,
    provider_id: &str,
    config: &ResolvedOAuthConfig,
    owner: Option<&str>,
    token: TokenResponse,
) -> Result<TokenResponse, OAuthProxyError> {
    if token.refresh_token.as_deref().is_none_or(str::is_empty) {
        if token.access_token.is_empty() || token.expires_in.is_some_and(|seconds| seconds <= 0) {
            return Err(connections::reconnect());
        }
        return Ok(token);
    }
    let store = connection_store(state, provider_id, config, owner)?;
    let id = store.adopt(token).await?;
    store
        .resolve(&id, |refresh| async move {
            refresh_provider(config, &refresh, None).await
        })
        .await
}

async fn accept_provider_authorization(
    state: &crate::state::State,
    provider: &str,
    config: &ResolvedOAuthConfig,
    owner: &str,
    token: TokenResponse,
) -> Result<TokenResponse, OAuthProxyError> {
    if token
        .refresh_token
        .as_deref()
        .is_some_and(|token| !token.is_empty())
    {
        connection_store(state, provider, config, Some(owner))?
            .reauthorize(token.clone())
            .await?;
    }
    adopt_response(state, provider, config, Some(owner), token).await
}

#[derive(Deserialize, ToSchema)]
pub struct AdoptConnectionRequest {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<i64>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
}

#[utoipa::path(
    post,
    path = "/oauth/connection/{provider_id}",
    tag = "oauth",
    params(("provider_id" = String, Path)),
    request_body = AdoptConnectionRequest,
    responses((status = 200, body = TokenResponse), (status = 400, body = ErrorResponse), (status = 401, body = ErrorResponse), (status = 503, body = ErrorResponse)),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /oauth/connection/:provider_id",
    skip(state, user, request)
)]
pub async fn adopt_connection(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(provider_id): Path<String>,
    Json(request): Json<AdoptConnectionRequest>,
) -> Result<Json<TokenResponse>, OAuthProxyError> {
    let owner = require_owner(&user)?;
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let config = require_provider_config(&provider_id, &configs)?;
    let config = resolve_connection_config(
        &state,
        &provider_id,
        config,
        &request.refresh_token,
        Some(&request.client_id),
    )
    .await?;
    let token = TokenResponse {
        access_token: request.access_token,
        refresh_token: Some(request.refresh_token),
        expires_in: request
            .expires_at
            .map(|expiry| expiry.saturating_sub(chrono::Utc::now().timestamp())),
        token_type: request.token_type,
        scope: request.scope,
        ..Default::default()
    };
    Ok(Json(
        adopt_response(&state, &provider_id, &config, Some(&owner), token).await?,
    ))
}

fn normalize_refresh_scope(scope: Option<&str>) -> Result<Option<String>, OAuthProxyError> {
    let Some(scope) = scope else { return Ok(None) };
    if scope.len() > 4096 {
        return Err(OAuthProxyError::new(
            StatusCode::BAD_REQUEST,
            "OAuth scope is too long",
        ));
    }
    let mut parts: Vec<_> = scope.split_whitespace().collect();
    parts.sort_unstable();
    parts.dedup();
    Ok((!parts.is_empty()).then(|| parts.join(" ")))
}

fn oauth_api_error(provider_id: &str, error: OAuthProxyError) -> crate::error::ApiError {
    if error.code == "oauth_reauthorization_required" {
        crate::error::ApiError::bad_request(format!(
            "Reconnect OAuth provider '{provider_id}' before running this workflow"
        ))
    } else if error.status == StatusCode::UNAUTHORIZED {
        crate::error::ApiError::unauthorized(error.message)
    } else if error.status == StatusCode::FORBIDDEN {
        crate::error::ApiError::forbidden(error.message)
    } else if error.status == StatusCode::BAD_REQUEST {
        crate::error::ApiError::bad_request(error.message)
    } else {
        crate::error::ApiError::service_unavailable(format!(
            "OAuth credentials for '{provider_id}' are unavailable: {}",
            error.message
        ))
    }
}

/// Incoming user credentials must belong to the authenticated hub account before
/// they become a delegated execution capability or a persistent event credential.
pub(crate) async fn authorize_incoming_oauth_tokens(
    state: &crate::state::State,
    user: &AppUser,
    tokens: &mut Option<HashMap<String, serde_json::Value>>,
) -> Result<(), crate::error::ApiError> {
    let Some(tokens) = tokens.as_mut().filter(|tokens| !tokens.is_empty()) else {
        return Ok(());
    };
    let owner = incoming_token_owner(user, tokens)?;
    for (provider, token) in tokens.iter_mut() {
        *token = resolve_oauth_token(state, provider, token, owner.as_deref())
            .await
            .map_err(|error| oauth_api_error(provider, error))?;
    }
    Ok(())
}

/// The account the incoming tokens resolve under. Executors and connected apps
/// have none, so every refresh token they pass must be an `fl_oauth_` handle,
/// and a replay may not refresh even those. All tokens are checked before any
/// is resolved.
fn incoming_token_owner(
    user: &AppUser,
    tokens: &HashMap<String, serde_json::Value>,
) -> Result<Option<String>, crate::error::ApiError> {
    if !matches!(user, AppUser::Executor(_) | AppUser::ConnectedApp(_)) {
        return require_owner(user)
            .map(Some)
            .map_err(|error| oauth_api_error("connection", error));
    }
    let mut carries_handle = false;
    for (provider, token) in tokens {
        match token
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
        {
            Some(refresh) if refresh.starts_with(connections::HANDLE_PREFIX) => {
                carries_handle = true;
            }
            Some(_) => return Err(oauth_api_error(provider, connections::reconnect())),
            None => {}
        }
    }
    if carries_handle {
        user.deny_shadow_execution(REFRESH_OAUTH_TOKENS)?;
    }
    Ok(None)
}

/// Resolves a sink's stored grant as its run-as user's. A sink without one has no grants
/// to use, so it is told to reconnect.
pub(crate) async fn resolve_sink_oauth_token(
    state: &crate::state::State,
    provider_id: &str,
    value: &serde_json::Value,
    run_as: Option<&str>,
) -> Result<serde_json::Value, crate::error::ApiError> {
    let Some(owner) = run_as else {
        return Err(oauth_api_error(provider_id, connections::reconnect()));
    };
    resolve_oauth_token(state, provider_id, value, Some(owner))
        .await
        .map_err(|error| oauth_api_error(provider_id, error))
}

async fn resolve_oauth_token(
    state: &crate::state::State,
    provider_id: &str,
    value: &serde_json::Value,
    owner: Option<&str>,
) -> Result<serde_json::Value, OAuthProxyError> {
    let refresh = value
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty());
    let expires_at = value.get("expires_at").and_then(serde_json::Value::as_i64);
    let access_token = value
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if refresh.is_none() {
        if access_token.is_empty()
            || expires_at.is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp() + 60)
        {
            return Err(connections::reconnect());
        }
        return Ok(value.clone());
    }
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let config = require_provider_config(provider_id, &configs)?;
    let config = resolve_connection_config(
        state,
        provider_id,
        config,
        refresh.unwrap(),
        value.get("client_id").and_then(serde_json::Value::as_str),
    )
    .await?;
    let token = TokenResponse {
        access_token: access_token.to_owned(),
        refresh_token: refresh.map(str::to_owned),
        expires_in: expires_at.map(|expiry| expiry.saturating_sub(chrono::Utc::now().timestamp())),
        token_type: value
            .get("token_type")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        scope: value
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        ..Default::default()
    };
    let resolved = adopt_response(state, provider_id, &config, owner, token).await?;
    let mut result = value.clone();
    result["access_token"] = resolved.access_token.into();
    result["refresh_token"] = serde_json::to_value(resolved.refresh_token).unwrap();
    result["expires_at"] = serde_json::to_value(
        resolved
            .expires_in
            .map(|seconds| chrono::Utc::now().timestamp().saturating_add(seconds)),
    )
    .unwrap();
    result["token_type"] = resolved
        .token_type
        .unwrap_or_else(|| "Bearer".to_owned())
        .into();
    if let Some(scope) = resolved.scope {
        result["scope"] = scope.into();
    }
    Ok(result)
}

/// Refresh an OAuth access token server-side using a refresh_token.
///
/// Used by the sink trigger path to refresh expired tokens before dispatching
/// scheduled executions. Reuses the same provider config and HTTP logic as
/// the `POST /oauth/refresh/{provider_id}` endpoint.
async fn refresh_provider(
    provider_config: &ResolvedOAuthConfig,
    refresh_token: &str,
    scope: Option<&str>,
) -> Result<TokenResponse, OAuthProxyError> {
    let client_id = require_client_id("configured provider", provider_config)?;
    refresh_provider_raw(provider_config, client_id, refresh_token, scope).await
}

async fn refresh_provider_raw(
    provider_config: &ResolvedOAuthConfig,
    client_id: &str,
    refresh_token: &str,
    scope: Option<&str>,
) -> Result<TokenResponse, OAuthProxyError> {
    let failed = || OAuthProxyError::new(StatusCode::BAD_GATEWAY, "OAuth provider refresh failed");
    let client = flow_like_types::reqwest::Client::builder()
        .timeout(OAUTH_REQUEST_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| failed())?;

    let response = match provider_config.auth_method {
        AuthMethod::BasicJson => {
            let client_secret = provider_config
                .client_secret
                .as_deref()
                .ok_or_else(failed)?;
            let credentials = flow_like_types::base64::Engine::encode(
                &flow_like_types::base64::engine::general_purpose::STANDARD,
                format!("{}:{}", client_id, client_secret),
            );

            let mut payload = serde_json::json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            });
            if let Some(scope) = scope {
                payload["scope"] = scope.into();
            }
            client
                .post(&provider_config.token_url)
                .header("Authorization", format!("Basic {}", credentials))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|_| failed())?
        }
        AuthMethod::FormPost => {
            let mut params = vec![
                ("grant_type", "refresh_token".to_string()),
                ("refresh_token", refresh_token.to_string()),
                ("client_id", client_id.to_string()),
            ];
            if let Some(client_secret) = provider_config.client_secret.as_ref() {
                params.push(("client_secret", client_secret.clone()));
            }
            if let Some(scope) = scope {
                params.push(("scope", scope.to_owned()));
            }

            client
                .post(&provider_config.token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .form(&params)
                .send()
                .await
                .map_err(|_| failed())?
        }
    };

    let status = response.status();
    let response_text = response.text().await.map_err(|_| failed())?;

    if !status.is_success() {
        let payload = parse_json_or_form_value(&response_text);
        if status.as_u16() == 400
            && matches!(
                payload.get("error").and_then(serde_json::Value::as_str),
                Some("invalid_scope" | "invalid_resource")
            )
        {
            return Err(OAuthProxyError::coded(
                StatusCode::BAD_REQUEST,
                "oauth_scope_rejected",
                "The provider rejected the requested OAuth scope",
            ));
        }
        if status.is_client_error()
            && matches!(
                payload.get("error").and_then(serde_json::Value::as_str),
                Some(
                    "invalid_client"
                        | "unauthorized_client"
                        | "unsupported_grant_type"
                        | "invalid_request"
                )
            )
        {
            return Err(OAuthProxyError::coded(
                StatusCode::BAD_REQUEST,
                "oauth_provider_rejected",
                "The OAuth provider rejected the refresh request configuration",
            ));
        }
        return Err(failed());
    }

    parse_token_response(&response_text).map_err(|_| failed())
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
        (status = 400, description = "Provider rejected the request or requires reconnection", body = ErrorResponse),
        (status = 401, description = "Hub sign-in required", body = ErrorResponse),
        (status = 404, description = "Provider not found", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /oauth/token/:provider_id", skip(state, request, user))]
pub async fn token_exchange(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(provider_id): Path<String>,
    Json(request): Json<TokenExchangeRequest>,
) -> Result<Json<TokenResponse>, OAuthProxyError> {
    let owner = require_owner(&user)?;
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let selected_config =
        select_client_config(&provider_id, provider_config, request.client_id.as_deref())?;
    let provider_config = &selected_config;
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
        let status_code = provider_status(status.as_u16());
        return Err(OAuthProxyError::new(
            status_code,
            format!(
                "Token exchange failed: {}",
                provider_error_message(&response_text)
            ),
        ));
    }

    let token_response = parse_token_response(&response_text)?;
    Ok(Json(
        accept_provider_authorization(
            &state,
            &provider_id,
            provider_config,
            &owner,
            token_response,
        )
        .await?,
    ))
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
        (status = 400, description = "Provider rejected the request or requires reconnection", body = ErrorResponse),
        (status = 401, description = "Hub sign-in required", body = ErrorResponse),
        (status = 403, description = "Forbidden: the caller is not a user account, or the run is a regression replay", body = ErrorResponse),
        (status = 404, description = "Provider not found", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /oauth/refresh/:provider_id", skip(state, request, user))]
pub async fn token_refresh(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(provider_id): Path<String>,
    Json(request): Json<TokenRefreshRequest>,
) -> Result<Json<TokenResponse>, OAuthProxyError> {
    // An executor already received this capability from a governed run. It may
    // resolve handles, but cannot register arbitrary provider credentials.
    let owner = match &user {
        AppUser::Executor(_)
            if request
                .refresh_token
                .starts_with(connections::HANDLE_PREFIX) =>
        {
            refuse_shadow_refresh(&user)?;
            None
        }
        _ => Some(require_owner(&user)?),
    };
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let config = require_provider_config(&provider_id, &configs)?;
    let config = resolve_connection_config(
        &state,
        &provider_id,
        config,
        &request.refresh_token,
        request.client_id.as_deref(),
    )
    .await?;
    let scope = normalize_refresh_scope(request.scope.as_deref())?;
    let store = connection_store(&state, &provider_id, &config, owner.as_deref())?;
    let id = store
        .adopt(TokenResponse {
            refresh_token: Some(request.refresh_token),
            ..Default::default()
        })
        .await?;
    let requested_scope = scope.as_deref();
    Ok(Json(
        store
            .resolve_scoped(&id, requested_scope, |refresh| async move {
                refresh_provider(&config, &refresh, requested_scope).await
            })
            .await?,
    ))
}

#[tracing::instrument(name = "POST /oauth/device/start/:provider_id", skip(state, request))]
pub async fn device_start(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<DeviceStartRequest>,
) -> Result<Json<DeviceStartResponse>, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let selected_config =
        select_client_config(&provider_id, provider_config, request.client_id.as_deref())?;
    let provider_config = &selected_config;
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
        let status_code = provider_status(status.as_u16());
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

#[tracing::instrument(
    name = "POST /oauth/device/poll/:provider_id",
    skip(state, request, user)
)]
pub async fn device_poll(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(provider_id): Path<String>,
    Json(request): Json<DevicePollRequest>,
) -> Result<Response, OAuthProxyError> {
    let owner = require_owner(&user)?;
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let selected_config =
        select_client_config(&provider_id, provider_config, request.client_id.as_deref())?;
    let provider_config = &selected_config;
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

    let status = provider_status(response.status().as_u16());
    let response_text = response
        .text()
        .await
        .map_err(|e| OAuthProxyError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let payload = parse_json_or_form_value(&response_text);
    if status.is_success() && payload.get("error").is_none() {
        let token = parse_token_response(&response_text)?;
        let token =
            accept_provider_authorization(&state, &provider_id, provider_config, &owner, token)
                .await?;
        return Ok((status, Json(token)).into_response());
    }
    Ok((status, Json(payload)).into_response())
}

#[tracing::instrument(name = "POST /oauth/userinfo/:provider_id", skip(state, request))]
pub async fn userinfo(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<UserInfoRequest>,
) -> Result<Response, OAuthProxyError> {
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
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
        let status_code = provider_status(status.as_u16());
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

#[tracing::instrument(name = "POST /oauth/revoke/:provider_id", skip(state, request, user))]
pub async fn revoke_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(provider_id): Path<String>,
    Json(request): Json<RevokeTokenRequest>,
) -> Result<StatusCode, OAuthProxyError> {
    let owner = require_owner(&user)?;
    let configs = get_oauth_configs(&state.secrets, &state.oauth_providers).await;
    let provider_config = require_provider_config(&provider_id, &configs)?;
    let provider_config =
        resolve_connection_config(&state, &provider_id, provider_config, &request.token, None)
            .await?;
    let token = if request.token.starts_with(connections::HANDLE_PREFIX) {
        let store = connection_store(&state, &provider_id, &provider_config, Some(&owner))?;
        let token = store.revoke(&request.token).await?;
        if provider_config.revoke_url.is_none() {
            return Ok(StatusCode::NO_CONTENT);
        }
        token
    } else {
        request.token
    };
    let revoke_url = provider_config.revoke_url.as_ref().ok_or_else(|| {
        OAuthProxyError::new(
            StatusCode::BAD_REQUEST,
            format!("Provider '{}' does not expose revoke endpoint", provider_id),
        )
    })?;
    let client_id = require_client_id(&provider_id, &provider_config)?;

    let mut params = vec![("token", token), ("client_id", client_id.to_string())];
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
        let status_code = provider_status(status.as_u16());
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
