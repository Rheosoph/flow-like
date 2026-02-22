use std::{collections::HashMap, sync::Arc};

use crate::{
    bit::{Bit, BitTypes},
    credentials::SharedCredentials,
    profile::Profile,
    utils::{http::HTTPClient, recursion::RecursionGuard},
};
use flow_like_types::{Result, sync::Mutex};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailProviderType {
    Ses,
    Sendgrid,
    Smtp,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct SmtpSettings {
    pub host_env: String,
    pub port_env: String,
    pub username_env: String,
    pub password_env: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct SendgridSettings {
    pub api_key_env: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct MailConfig {
    pub provider: MailProviderType,
    pub from_email: String,
    pub from_name: String,
    pub smtp: Option<SmtpSettings>,
    pub sendgrid: Option<SendgridSettings>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct AlertingConfig {
    pub mail: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct UserTier {
    pub max_non_visible_projects: i32,
    pub max_remote_executions: i32,
    pub execution_tier: String,
    pub max_total_size: i64,
    pub max_llm_cost: i32,
    pub max_llm_calls: Option<i32>,
    pub llm_tiers: Vec<String>,
    pub product_id: Option<String>,
}

pub type UserTiers = HashMap<String, UserTier>;

fn default_secure() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct Hub {
    pub name: String,
    pub description: String,
    pub thumbnail: Option<String>,
    pub icon: Option<String>,
    pub authentication: Option<Authentication>,
    pub features: Features,
    pub hubs: Vec<String>,
    pub provider: Option<String>,
    pub domain: String,
    #[serde(default = "default_secure")]
    pub secure: bool,
    pub region: Option<String>,
    pub terms_of_service: String,
    pub signaling: Option<Vec<String>>,
    pub cdn: Option<String>,
    pub app: Option<String>,
    pub web: Option<String>,
    pub mail: Option<MailConfig>,
    pub alerting: Option<AlertingConfig>,
    pub legal_notice: String,
    pub privacy_policy: String,
    pub contact: Contact,
    pub max_users_prototype: Option<i32>,
    pub default_user_plan: Option<String>,
    pub environment: Environment,
    pub tiers: UserTiers,
    #[serde(default)]
    pub lookup: Lookup,
    /// OAuth provider configurations
    #[serde(default)]
    pub oauth_providers: OAuthProviderConfigs,

    /// Supported server-side event sinks (e.g., discord, telegram, cron, http)
    /// If None, defaults to basic sinks like http, webhook, cron
    #[serde(default)]
    pub supported_sinks: Option<SupportedSinks>,

    /// WASM registry configuration
    #[serde(default)]
    pub wasm_registry_config: WasmRegistryConfig,

    #[serde(skip)]
    recursion_guard: Option<Arc<Mutex<RecursionGuard>>>,

    #[serde(skip)]
    http_client: Option<Arc<HTTPClient>>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize, PartialEq)]
pub enum Environment {
    Development,
    Production,
    Staging,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct Authentication {
    pub variant: String,
    pub openid: Option<OpenIdConfig>,
    pub oauth2: Option<OAuth2Config>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct Lookup {
    pub email: bool,
    pub name: bool,
    pub username: bool,
    pub preferred_username: bool,
    pub avatar: bool,
    pub additional_information: bool,
    pub description: bool,
    pub created_at: bool,
}

impl Default for Lookup {
    fn default() -> Self {
        Self {
            email: false,
            username: false,
            name: true,
            preferred_username: true,
            avatar: true,
            additional_information: true,
            description: true,
            created_at: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct OpenIdProxy {
    pub enabled: bool,
    pub authorize: Option<String>,
    pub token: Option<String>,
    pub userinfo: Option<String>,
    pub revoke: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct CognitoConfig {
    pub user_pool_id: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct OpenIdConfig {
    pub authority: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub post_logout_redirect_uri: Option<String>,
    pub response_type: Option<String>,
    pub scope: Option<String>,
    pub discovery_url: Option<String>,
    pub user_info_url: Option<String>,
    pub jwks_url: String,
    pub proxy: Option<OpenIdProxy>,
    pub cognito: Option<CognitoConfig>,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct OAuth2Config {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: String,
}

/// OAuth provider configuration from the config file.
/// This is used to configure OAuth providers centrally.
/// The client_secret is resolved from environment variables at build time for providers that need it.
#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct OAuthProviderConfig {
    /// Display name shown to users
    pub name: String,
    /// The client ID (public, not secret)
    #[serde(default)]
    pub client_id: String,
    /// Environment variable name containing the client secret (resolved at build time)
    /// If null, no secret is needed (PKCE-based flow)
    pub client_secret_env: Option<String>,
    /// The resolved client secret (populated at build time from the env var)
    #[serde(default)]
    pub client_secret: Option<String>,
    /// OAuth authorization endpoint URL
    pub auth_url: String,
    /// OAuth token endpoint URL
    pub token_url: String,
    /// Base OAuth scopes (node-specific scopes will be added by the frontend)
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Whether PKCE is required
    #[serde(default)]
    pub pkce_required: bool,
    /// Whether this provider requires the secret proxy for token exchange
    /// If true, token exchange requests go through the API server which adds the secret
    #[serde(default)]
    pub requires_secret_proxy: bool,
    /// Optional: URL for token revocation
    pub revoke_url: Option<String>,
    /// Optional: URL for user info endpoint
    pub userinfo_url: Option<String>,
    /// Optional: Device authorization URL for device flow
    pub device_auth_url: Option<String>,
    /// Whether to use device flow
    #[serde(default)]
    pub use_device_flow: bool,
    #[serde(default)]
    pub use_implicit_flow: bool,
    /// Optional: Audience claim for token validation
    pub audience: Option<String>,
}

pub type OAuthProviderConfigs = HashMap<String, OAuthProviderConfig>;

/// Configuration for supported server-side event sinks.
/// When a hub is deployed, only sinks listed here will be available for server-side execution.
/// The desktop app always has access to all sinks.
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default)]
pub struct SupportedSinks {
    /// HTTP/REST API endpoint sink
    #[serde(default)]
    pub http: bool,
    /// Incoming webhook from external service
    #[serde(default)]
    pub webhook: bool,
    /// Cron scheduled trigger
    #[serde(default)]
    pub cron: bool,
    /// MQTT message broker
    #[serde(default)]
    pub mqtt: bool,
    /// GitHub repository webhook
    #[serde(default)]
    pub github: bool,
    /// RSS feed polling
    #[serde(default)]
    pub rss: bool,
    /// Discord bot integration
    #[serde(default)]
    pub discord: bool,
    /// Slack bot integration
    #[serde(default)]
    pub slack: bool,
    /// Telegram bot integration
    #[serde(default)]
    pub telegram: bool,
    /// Email/IMAP polling
    #[serde(default)]
    pub email: bool,
}

impl SupportedSinks {
    /// Returns a list of enabled sink types as strings
    pub fn enabled_sinks(&self) -> Vec<&'static str> {
        let mut sinks = Vec::new();
        if self.http {
            sinks.push("http");
        }
        if self.webhook {
            sinks.push("webhook");
        }
        if self.cron {
            sinks.push("cron");
        }
        if self.mqtt {
            sinks.push("mqtt");
        }
        if self.github {
            sinks.push("github");
        }
        if self.rss {
            sinks.push("rss");
        }
        if self.discord {
            sinks.push("discord");
        }
        if self.slack {
            sinks.push("slack");
        }
        if self.telegram {
            sinks.push("telegram");
        }
        if self.email {
            sinks.push("email");
        }
        sinks
    }

    /// Returns true if the given sink type is supported
    pub fn is_supported(&self, sink_type: &str) -> bool {
        match sink_type.to_lowercase().as_str() {
            "http" | "api" => self.http,
            "webhook" => self.webhook,
            "cron" => self.cron,
            "mqtt" => self.mqtt,
            "github" => self.github,
            "rss" => self.rss,
            "discord" => self.discord,
            "slack" => self.slack,
            "telegram" => self.telegram,
            "email" => self.email,
            _ => false,
        }
    }

    /// Default configuration for basic server setups (http, webhook, cron)
    pub fn basic() -> Self {
        Self {
            http: true,
            webhook: true,
            cron: true,
            ..Default::default()
        }
    }

    /// Configuration with all sinks enabled
    pub fn all() -> Self {
        Self {
            http: true,
            webhook: true,
            cron: true,
            mqtt: true,
            github: true,
            rss: true,
            discord: true,
            slack: true,
            telegram: true,
            email: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemberLeavePolicy {
    /// Remove packages added by the departing member from the app
    Remove,
    /// Mark packages as stale — frozen version, no updates, cannot be placed on new boards
    Stale,
}

impl Default for MemberLeavePolicy {
    fn default() -> Self {
        Self::Stale
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct WasmRegistryConfig {
    /// What happens to app packages when the member who added them leaves the app
    #[serde(default)]
    pub on_member_leave: MemberLeavePolicy,
}

impl Default for WasmRegistryConfig {
    fn default() -> Self {
        Self {
            on_member_leave: MemberLeavePolicy::Stale,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct Features {
    pub model_hosting: bool,
    pub flow_hosting: bool,
    pub governance: bool,
    pub ai_act: bool,
    pub unauthorized_read: bool,
    pub admin_interface: bool,
    pub premium: bool,
    #[serde(default)]
    pub wasm_registry: bool,
    #[serde(default)]
    pub wasm_server_compilation: bool,
    #[serde(default)]
    pub app_package_linking: bool,
    #[serde(default)]
    pub wasm_package_user_management: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct Contact {
    pub name: String,
    pub email: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BitSearchQuery {
    pub search: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub bit_types: Option<Vec<BitTypes>>,
}

impl BitSearchQuery {
    pub fn builder() -> Self {
        Self {
            search: None,
            limit: None,
            offset: None,
            bit_types: None,
        }
    }

    pub fn with_search(mut self, search: &str) -> Self {
        self.search = Some(search.to_string());
        self
    }

    pub fn with_limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn with_offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn with_bit_types(mut self, bit_types: Vec<BitTypes>) -> Self {
        self.bit_types = Some(bit_types);
        self
    }

    pub fn build(self) -> Self {
        self
    }
}

impl Hub {
    fn http_client(&self) -> Arc<HTTPClient> {
        self.http_client.clone().unwrap()
    }

    pub async fn new(url: &str, http_client: Arc<HTTPClient>) -> Result<Hub> {
        let mut url = String::from(url);
        if !url.starts_with("https://") {
            url = format!("https://{}", url);
        }

        if !url.ends_with('/') {
            url.push('/');
        }

        let url = match Url::parse(&url) {
            Ok(url) => url,
            Err(_e) => {
                return Err(flow_like_types::Error::msg("Invalid URL"));
            }
        };

        // TODO Cache this.
        // We should implement a global Cache anyways, best with support for reqwest
        let hub_info_url = url.join("api/v1")?;
        let request = http_client.client().get(hub_info_url.clone()).build()?;
        let mut info: Hub = http_client.hashed_request(request).await?;
        info.recursion_guard = Some(RecursionGuard::new(vec![url.as_ref()]));
        info.http_client = Some(http_client);
        Ok(info)
    }

    fn construct_url(&self, path: &str) -> Result<Url> {
        let mut url = if !self.domain.starts_with("https://") {
            format!("https://{}", self.domain)
        } else {
            self.domain.clone()
        };

        if !url.ends_with("/") {
            url.push('/');
        }

        if path.starts_with('/') {
            url.push_str(&path[1..]);
        } else {
            url.push_str(path);
        }
        let url = Url::parse(&url)
            .map_err(|e| flow_like_types::Error::msg(format!("Invalid URL: {}", e)))?;

        Ok(url)
    }

    pub async fn shared_credentials(&self, token: &str, app_id: &str) -> Result<SharedCredentials> {
        let presign_path = format!("api/v1/apps/{}/invoke/presign", app_id);

        let presign_url = self.construct_url(&presign_path)?;

        let auth_val = if token.starts_with("pat_") {
            token.to_string()
        } else if token.starts_with("Bearer ") {
            token.to_string()
        } else {
            format!("Bearer {}", token)
        };

        let client = self.http_client().client();

        let request = client
            .get(presign_url.clone())
            .header("Authorization", &auth_val)
            .build()
            .map_err(flow_like_types::Error::from)?;

        let resp = client
            .execute(request)
            .await
            .map_err(flow_like_types::Error::from)?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(flow_like_types::Error::from)?;

        if !status.is_success() {
            return Err(flow_like_types::Error::msg(format!(
                "presign failed: status={} body={}",
                status, body_text
            )));
        }

        let shared_credentials: SharedCredentials = flow_like_types::json::from_str(&body_text)
            .map_err(|e| flow_like_types::Error::msg(format!("JSON parse error: {}", e)))?;

        Ok(shared_credentials)
    }

    pub async fn get_bit(&self, bit_id: &str) -> Result<Bit> {
        let bit_url = self.construct_url(&format!("api/v1/bit/{}", bit_id))?;
        let request = self.http_client().client().get(bit_url).build()?;
        let bit = self.http_client().hashed_request::<Bit>(request).await;
        if let Ok(bit) = bit {
            return Ok(bit);
        }

        let dependency_hubs = self.get_dependency_hubs().await?;
        for hub in dependency_hubs {
            let bit = Box::pin(hub.get_bit(bit_id)).await;
            match bit {
                Ok(bit) => return Ok(bit),
                Err(_) => continue,
            }
        }

        Err(flow_like_types::Error::msg("Bit not found"))
    }

    pub async fn set_recursion_guard(&mut self, guard: Arc<Mutex<RecursionGuard>>) {
        self.recursion_guard = Some(guard);
        if let Some(ref guard) = self.recursion_guard {
            guard.lock().await.insert(&self.domain);
        }
    }

    pub async fn search_bit(&self, query: &BitSearchQuery) -> Result<Vec<Bit>> {
        let type_bits_url = self.construct_url("api/v1/bit")?;

        let request = self
            .http_client()
            .client()
            .post(type_bits_url)
            .json(query)
            .build()?;
        let mut bits = self
            .http_client()
            .hashed_request::<Vec<Bit>>(request)
            .await?;
        let dependency_hubs = self.get_dependency_hubs().await?;

        for hub in dependency_hubs {
            let hub_models = Box::pin(hub.search_bit(query)).await?;
            bits.extend(hub_models);
        }

        Ok(bits)
    }

    pub async fn get_bit_dependencies(&self, bit_id: &str) -> Result<Vec<Bit>> {
        let dependencies_url =
            self.construct_url(&format!("api/v1/bit/{}/dependencies", bit_id))?;
        let request = self.http_client().client().get(dependencies_url).build()?;
        let bits = self
            .http_client()
            .hashed_request::<Vec<Bit>>(request)
            .await?;

        Ok(bits)
    }

    pub async fn get_profiles(&self) -> Result<Vec<Profile>> {
        let profiles_url = self.construct_url("api/v1/info/profiles")?;
        let request = self.http_client().client().get(profiles_url).build()?;
        let bits = self
            .http_client()
            .hashed_request::<Vec<Profile>>(request)
            .await?;
        let bits = bits
            .into_iter()
            .map(|mut bit| {
                bit.hub = self.domain.clone();
                bit
            })
            .collect();
        Ok(bits)
    }

    // should be optimized
    pub async fn get_dependency_hubs(&self) -> Result<Vec<Hub>> {
        let recursion_guard = if let Some(guard) = &self.recursion_guard {
            guard.clone()
        } else {
            RecursionGuard::new(vec![&self.domain])
        };

        let mut hubs = vec![];
        for hub in &self.hubs {
            let guard = recursion_guard.clone();
            let mut guard = guard.lock().await;

            if hub == &self.domain {
                continue;
            }

            if guard.contains(hub) {
                continue;
            }

            guard.insert(hub);
            drop(guard);

            let hub = Hub::new(hub, self.http_client()).await;
            let mut hub = match hub {
                Ok(hub) => hub,
                Err(_) => continue,
            };
            hub.set_recursion_guard(recursion_guard.clone()).await;
            hubs.push(hub);
        }
        Ok(hubs)
    }
}
