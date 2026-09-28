use std::{collections::HashMap, sync::Arc};

use crate::{
    bit::{Bit, BitTypes},
    credentials::SharedCredentials,
    flow::execution::UserExecutionContext,
    profile::Profile,
    utils::{http::HTTPClient, recursion::RecursionGuard},
};
use flow_like_types::{Result, authorization::AuthorizationError, sync::Mutex};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

mod payments;
mod standalone;
pub use payments::{
    PaymentFeeBasis, PaymentLegalText, PaymentTaxMode, PaymentsConfig, valid_product_tax_code,
};
pub use standalone::StandaloneConfig;

#[derive(Clone, Copy, Debug, Serialize, JsonSchema, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MailProviderType {
    Ses,
    Sendgrid,
    Smtp,
    #[serde(rename = "azure_communication_services", alias = "acs_email")]
    AzureCommunicationServices,
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

fn default_false() -> bool {
    false
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushNotificationProviderType {
    Fcm,
    AwsSns,
    AzureNotificationHubs,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, Deserialize)]
pub struct FcmPushNotificationsConfig {
    pub project_id: String,
    pub service_account_json_env: String,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, Deserialize)]
pub struct AwsSnsPushNotificationsConfig {
    pub android_platform_application_arn_env: String,
    pub ios_platform_application_arn_env: String,
    pub region_env: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, Deserialize)]
pub struct AzureNotificationHubsPushNotificationsConfig {
    pub namespace: String,
    pub hub_name: String,
    pub sas_key_name_env: String,
    pub sas_key_value_env: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct PushNotificationsConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_mobile: bool,
    #[serde(default)]
    pub allow_desktop: bool,
    pub provider: Option<PushNotificationProviderType>,
    pub channel_id: Option<String>,
    pub fcm: Option<FcmPushNotificationsConfig>,
    pub aws_sns: Option<AwsSnsPushNotificationsConfig>,
    pub azure_notification_hubs: Option<AzureNotificationHubsPushNotificationsConfig>,
}

impl Default for PushNotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_mobile: true,
            allow_desktop: false,
            provider: None,
            channel_id: None,
            fcm: None,
            aws_sns: None,
            azure_notification_hubs: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize)]
pub struct UserTier {
    pub max_non_visible_projects: i32,
    pub max_remote_executions: i32,
    /// Monthly cloud workflow runtime in milliseconds; negative means unlimited.
    #[serde(default = "unlimited_quota")]
    pub max_runtime_ms: i64,
    #[serde(default = "unlimited_quota_i32")]
    pub max_concurrent_executions: i32,
    /// Monthly hosted AI allowance in EUR millionths, including serving costs.
    #[serde(default = "unlimited_quota")]
    pub max_ai_cost_micros: i64,
    pub execution_tier: String,
    pub max_total_size: i64,
    pub max_llm_cost: i32,
    pub max_llm_calls: Option<i32>,
    pub llm_tiers: Vec<String>,
    pub product_id: Option<String>,
}

fn unlimited_quota_i32() -> i32 {
    -1
}

fn unlimited_quota() -> i64 {
    -1
}

pub type UserTiers = HashMap<String, UserTier>;

/// Decides what users see when they hit a plan limit: a self-service upgrade
/// flow (consumer) or a "contact us" card (enterprise deployments).
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversionMode {
    #[default]
    Consumer,
    Enterprise,
}

/// Marketing metadata for a tier, keyed by tier id in `ConversionConfig`.
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, Default)]
pub struct TierDisplay {
    pub monthly_price_cents: Option<i64>,
    pub annual_price_cents: Option<i64>,
    pub currency: Option<String>,
    /// Name shown instead of the raw tier key (e.g. "Starter" for FREE)
    pub display_name: Option<String>,
    /// One-line value proposition under the tier name
    pub tagline: Option<String>,
    /// Curated feature bullets shown in addition to the derived limit facts
    #[serde(default)]
    pub features: Vec<String>,
    /// Marks the recommended tier — gets the "Most popular" treatment
    #[serde(default)]
    pub highlight: bool,
    /// Badge text overriding the default highlight label
    pub badge: Option<String>,
}

/// Upgrade / conversion experience configuration
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct ConversionConfig {
    /// When false, plan-limit errors surface as plain messages without the
    /// upgrade dialog
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: ConversionMode,
    /// Headline override for the upgrade dialog
    pub headline: Option<String>,
    /// Supporting line under the headline
    pub subheadline: Option<String>,
    /// Contact used in enterprise mode and for the enterprise tier CTA;
    /// falls back to the hub contact when unset
    pub contact: Option<Contact>,
    /// Custom message shown on the enterprise contact card
    pub contact_message: Option<String>,
    /// Per-tier marketing metadata keyed by tier id (FREE, PREMIUM, ...)
    #[serde(default)]
    pub tier_display: HashMap<String, TierDisplay>,
}

impl Default for ConversionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: ConversionMode::default(),
            headline: None,
            subheadline: None,
            contact: None,
            contact_message: None,
            tier_display: HashMap::new(),
        }
    }
}

fn default_secure() -> bool {
    true
}

fn default_cloudflare_ice_ttl_seconds() -> u32 {
    4 * 60 * 60
}

/// Selects the service that issues short-lived STUN and TURN configuration for
/// realtime clients. Provider secrets are resolved by the API from references
/// in the secret store and never belong in the hub JSON itself.
#[derive(Clone, Debug, Serialize, JsonSchema, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum RealtimeIceConfig {
    Cloudflare {
        /// Secret-store reference containing the Cloudflare TURN key identifier.
        turn_key_id_secret_ref: String,
        /// Secret-store reference containing the bearer key returned with the TURN key.
        turn_key_api_token_secret_ref: String,
        /// Lifetime of each client credential. Cloudflare accepts at most 48 hours.
        #[serde(default = "default_cloudflare_ice_ttl_seconds")]
        #[schemars(range(min = 300, max = 172800))]
        ttl_seconds: u32,
    },
}

#[derive(Clone, Debug, Default, Serialize, JsonSchema, Deserialize, PartialEq, Eq)]
pub struct RealtimeConfig {
    /// Omit this field to retain the WebRTC library's built-in ICE defaults.
    pub ice: Option<RealtimeIceConfig>,
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
    /// Realtime transport configuration. Signaling remains configured separately.
    #[serde(default)]
    pub realtime: RealtimeConfig,
    #[serde(default)]
    pub standalone: StandaloneConfig,
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

    /// Audit trail configuration
    #[serde(default)]
    pub audit: AuditConfig,

    #[serde(default)]
    pub payments: PaymentsConfig,

    /// Push notification provider configuration
    #[serde(default)]
    pub push_notifications: PushNotificationsConfig,

    /// Fork-an-app feature configuration
    #[serde(default)]
    pub forking: ForkingConfig,

    /// Upgrade / conversion experience configuration
    #[serde(default)]
    pub conversion: ConversionConfig,

    /// Where apps' Flow-Like storage files are served. A widget can only be
    /// granted `{origin}{path_prefix}{appId}/` on these origins, never the
    /// whole origin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widget_storage: Vec<HubWidgetStorage>,

    #[serde(skip)]
    recursion_guard: Option<Arc<Mutex<RecursionGuard>>>,

    #[serde(skip)]
    http_client: Option<Arc<HTTPClient>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq)]
pub struct HubWidgetStorage {
    /// `scheme://host` the content bucket is served from
    pub origin: String,
    /// Path before the app id, starting and ending with `/`
    pub path_prefix: String,
}

/// Fork-an-app feature config. Controls quotas and the unauthenticated-fork
/// path. Defaults are conservative: feature on, 1 GB / 10k file cap, no
/// anonymous forking.
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct ForkingConfig {
    /// Global kill switch — when false, every fork endpoint refuses
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Hard cap on the total bytes copied in a single fork
    #[serde(default = "default_fork_max_size_bytes")]
    pub max_size_bytes: u64,
    /// Hard cap on the number of objects copied in a single fork
    #[serde(default = "default_fork_max_file_count")]
    pub max_file_count: u64,
    /// Whether anonymous (unauthenticated) callers may fork a public+free app
    /// to an offline (desktop) destination
    #[serde(default)]
    pub allow_unauthenticated_to_offline: bool,
}

fn default_fork_max_size_bytes() -> u64 {
    1_073_741_824
}

fn default_fork_max_file_count() -> u64 {
    10_000
}

impl Default for ForkingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_size_bytes: default_fork_max_size_bytes(),
            max_file_count: default_fork_max_file_count(),
            allow_unauthenticated_to_offline: false,
        }
    }
}

/// How much of the mutation surface the audit trail records. Each level includes
/// everything below it. Execution lifecycle records additionally follow
/// `AuditConfig::log_executions`.
#[derive(
    Debug, Serialize, Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum AuditLevel {
    /// Identity, access, credentials, publication, deletions and platform administration.
    Minimal,
    /// Minimal plus content changes: boards, events, pages, widgets, files, tables, settings.
    #[default]
    Standard,
    /// Standard plus every request attempt and outcome, editor commands, graph row
    /// writes, file read grants and execution lifecycle records.
    Verbose,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct AuditConfig {
    /// Master switch. When false, no audit entries are recorded.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which action families are recorded. Defaults to `standard`.
    #[serde(default)]
    pub level: AuditLevel,
    /// Whether to capture client IP addresses in audit entries (GDPR consideration)
    #[serde(default)]
    pub log_ip: bool,
    /// Reverse proxies under the deployment's control that append to
    /// `X-Forwarded-For`. The recorded client IP is taken that many entries from
    /// the right, which a client cannot forge. Unset records the leftmost entry,
    /// which the client chooses.
    #[serde(default)]
    pub trusted_proxy_hops: Option<u32>,
    /// Refuse to run without keys: the API without an entry key, the audit worker
    /// without an audit signing key.
    #[serde(default)]
    pub require_signing: bool,
    /// Record execution lifecycle transitions at any level.
    /// The `verbose` level records them regardless of this switch.
    #[serde(default)]
    pub log_executions: bool,
    /// How long each part of the trail is kept, and when the audit worker seals.
    #[serde(default)]
    pub retention: AuditRetention,
}

/// Retention, sealing and signing cadence. Evidence is pruned only once its month is
/// archived to the audit bucket, so a deployment without `AUDIT_BUCKET` keeps every
/// evidence record; activity records are deleted after their window.
#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, PartialEq, Eq)]
pub struct AuditRetention {
    /// Days security and content records stay in the database after their month
    /// closes. Older months are read from the monthly archive.
    #[serde(default = "default_evidence_hot_days")]
    pub evidence_hot_days: u32,
    /// Informational horizon written into archive manifests: the end of this many
    /// calendar years after the month. The bucket's retention lock enforces it.
    #[serde(default = "default_archive_years")]
    pub archive_years_after_year_end: u32,
    /// Days records of verbose-only actions stay in the database. They are never
    /// archived.
    #[serde(default = "default_activity_days")]
    pub activity_days: u32,
    /// Minimum days for execution records of apps assessed as high-risk AI systems
    /// (EU AI Act art. 19 and 26(6)).
    #[serde(default = "default_high_risk_activity_days")]
    pub high_risk_activity_days: u32,
    /// Days a recorded client IP is kept before it is removed from its record.
    #[serde(default = "default_ip_days")]
    pub ip_days: u32,
    /// Days record details are kept. Unset keeps them as long as the record.
    #[serde(default)]
    pub details_days: Option<u32>,
    /// Seal a chain once this many records are pending.
    #[serde(default = "default_seal_after_records")]
    pub seal_after_records: u32,
    /// Seal a chain once its oldest pending record is this old.
    #[serde(default = "default_seal_after_seconds")]
    pub seal_after_seconds: u32,
    /// Largest seal the worker writes in one transaction.
    #[serde(default = "default_max_records_per_seal")]
    pub max_records_per_seal: u32,
    /// Sign an epoch once the oldest unanchored seal is this old, or earlier when 20,000
    /// seals wait. Each epoch is one request to the key service, so this sets its cost:
    /// 300 s means at most 12 routine signatures an hour.
    #[serde(default = "default_epoch_interval_seconds")]
    pub epoch_interval_seconds: u32,
    /// Log an alert when the oldest pending record is older than this.
    #[serde(default = "default_pending_alert_seconds")]
    pub pending_alert_seconds: u32,
    /// Days after a month closes before it is archived.
    #[serde(default = "default_archive_grace_days")]
    pub archive_grace_days: u32,
}

fn default_evidence_hot_days() -> u32 {
    396
}

fn default_archive_years() -> u32 {
    3
}

fn default_activity_days() -> u32 {
    90
}

fn default_high_risk_activity_days() -> u32 {
    183
}

fn default_ip_days() -> u32 {
    7
}

fn default_seal_after_records() -> u32 {
    500
}

fn default_seal_after_seconds() -> u32 {
    300
}

fn default_max_records_per_seal() -> u32 {
    1000
}

fn default_epoch_interval_seconds() -> u32 {
    300
}

fn default_pending_alert_seconds() -> u32 {
    900
}

fn default_archive_grace_days() -> u32 {
    3
}

impl Default for AuditRetention {
    fn default() -> Self {
        Self {
            evidence_hot_days: default_evidence_hot_days(),
            archive_years_after_year_end: default_archive_years(),
            activity_days: default_activity_days(),
            high_risk_activity_days: default_high_risk_activity_days(),
            ip_days: default_ip_days(),
            details_days: None,
            seal_after_records: default_seal_after_records(),
            seal_after_seconds: default_seal_after_seconds(),
            max_records_per_seal: default_max_records_per_seal(),
            epoch_interval_seconds: default_epoch_interval_seconds(),
            pending_alert_seconds: default_pending_alert_seconds(),
            archive_grace_days: default_archive_grace_days(),
        }
    }
}

fn default_true() -> bool {
    true
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: AuditLevel::default(),
            log_ip: false,
            trusted_proxy_hops: None,
            require_signing: false,
            log_executions: false,
            retention: AuditRetention::default(),
        }
    }
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
    /// Exact token issuer (`iss`) accepted by the API. When omitted for an
    /// existing deployment, `authority` remains the compatibility fallback.
    pub issuer: Option<String>,
    pub authority: Option<String>,
    pub client_id: Option<String>,
    /// Exact token audience accepted by the API. Defaults to `client_id`,
    /// which is the correct target for OIDC ID tokens and Cognito tokens.
    pub audience: Option<String>,
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
/// Desktop availability is defined by each sink type.
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
    /// Email received at an address issued by the server
    #[serde(default)]
    pub inbound_email: bool,
    /// Microsoft Teams bot webhook
    #[serde(default)]
    pub teams: bool,
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
        if self.inbound_email {
            sinks.push("inbound_email");
        }
        if self.teams {
            sinks.push("teams");
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
            "inbound_email" => self.inbound_email,
            "teams" => self.teams,
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
            inbound_email: true,
            teams: true,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MemberLeavePolicy {
    /// Remove packages added by the departing member from the app
    Remove,
    /// Mark packages as stale — frozen version, no updates, cannot be placed on new boards
    #[default]
    Stale,
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
    #[serde(default)]
    pub telemetry: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct Contact {
    pub name: String,
    pub email: String,
    pub url: String,
}

impl Contact {
    /// Email when present, otherwise the contact URL — for user-facing prose.
    pub fn preferred_reference(&self) -> &str {
        if self.email.is_empty() {
            &self.url
        } else {
            &self.email
        }
    }
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

/// Turn a hub reference into a scheme-qualified origin.
///
/// Hubs are persisted as bare domains (`api.flow-like.com`) with the scheme
/// carried separately in `secure`, so every consumer that builds a URL from a
/// hub has to re-attach it. Returns `None` for blank input.
pub fn hub_origin(hub: &str, secure: bool) -> Option<String> {
    let hub = hub.trim().trim_end_matches('/');
    if hub.is_empty() {
        return None;
    }

    if hub.contains("://") {
        return Some(hub.to_string());
    }

    let scheme = if secure { "https" } else { "http" };
    Some(format!("{scheme}://{hub}"))
}

const HUB_ERROR_MESSAGE_BYTES: usize = 512;

/// `{origin}/api/v1/{segments}` for a hub given as a host or URL, with or
/// without the `/api/v1` base. Segments are percent-encoded individually.
pub fn hub_api_url(hub: &str, secure: bool, segments: &[&str]) -> Result<Url> {
    let origin = hub_origin(hub, secure)
        .ok_or_else(|| flow_like_types::anyhow!("No hub API URL is configured"))?;
    let mut url = Url::parse(&origin)
        .map_err(|error| flow_like_types::anyhow!("Invalid hub API URL '{origin}': {error}"))?;
    let needs_api_prefix = !url.path().trim_end_matches('/').ends_with("/api/v1");
    let mut path = url
        .path_segments_mut()
        .map_err(|_| flow_like_types::anyhow!("Hub API URL '{origin}' cannot carry a path"))?;
    path.pop_if_empty();
    if needs_api_prefix {
        path.extend(["api", "v1"]);
    }
    path.extend(segments);
    drop(path);
    Ok(url)
}

/// Which hub responses [`send_hub_request`] repeats, how often, and how long
/// it may wait between attempts in total.
#[derive(Clone, Copy, Debug)]
pub struct HubRetry {
    pub retryable: fn(flow_like_types::reqwest::StatusCode) -> bool,
    pub retries: u32,
    pub max_wait: std::time::Duration,
}

/// Send a hub request, repeating a retryable response after its `Retry-After`
/// seconds (else 1, 2, 4 … s) while attempts and total wait stay within `retry`.
/// `build` runs once per attempt so per-request proofs are never replayed.
/// The last response is returned whatever its status.
pub async fn send_hub_request<F, Fut>(
    client: &flow_like_types::reqwest::Client,
    retry: HubRetry,
    mut build: F,
) -> Result<flow_like_types::reqwest::Response>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<flow_like_types::reqwest::Request>>,
{
    let mut waited = std::time::Duration::ZERO;
    let mut attempt = 0;
    loop {
        let response = client.execute(build().await?).await?;
        let status = response.status();
        if status.is_success() || attempt >= retry.retries || !(retry.retryable)(status) {
            return Ok(response);
        }
        let delay = retry_after(response.headers())
            .unwrap_or_else(|| std::time::Duration::from_secs(1 << attempt.min(5)));
        if waited + delay > retry.max_wait {
            return Ok(response);
        }
        drop(response);
        flow_like_types::tokio::time::sleep(delay).await;
        waited += delay;
        attempt += 1;
    }
}

/// `Retry-After` given in seconds.
pub fn retry_after(
    headers: &flow_like_types::reqwest::header::HeaderMap,
) -> Option<std::time::Duration> {
    headers
        .get(flow_like_types::reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
        .map(std::time::Duration::from_secs)
}

/// Error for a failed hub response carrying the API's public message
/// (`{"error":{"message":…}}`, else the raw body), bounded to 512 bytes.
pub async fn hub_response_error(
    operation: &str,
    response: flow_like_types::reqwest::Response,
) -> flow_like_types::Error {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    match hub_error_message(&body) {
        message if message.is_empty() => {
            flow_like_types::anyhow!("{operation} was rejected by the server ({status})")
        }
        message => {
            flow_like_types::anyhow!("{operation} was rejected by the server ({status}): {message}")
        }
    }
}

fn hub_error_message(body: &str) -> String {
    let message = flow_like_types::json::from_str::<flow_like_types::Value>(body)
        .ok()
        .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.trim().to_owned());
    let mut end = message.len().min(HUB_ERROR_MESSAGE_BYTES);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_owned()
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

        url.push_str(path.strip_prefix('/').unwrap_or(path));
        let url = Url::parse(&url)
            .map_err(|e| flow_like_types::Error::msg(format!("Invalid URL: {}", e)))?;

        Ok(url)
    }

    /// Resolve the caller's execution identity for an app: subject, role,
    /// permissions and attributes exactly as the server would grant them.
    ///
    /// A hosted app executed on the desktop has to ask for this rather than
    /// assume owner rights, or the same board answers `Has Permission`
    /// differently locally than in the cloud.
    pub async fn execution_context(
        &self,
        token: &str,
        app_id: &str,
    ) -> Result<UserExecutionContext> {
        let context_url = self.construct_url(&format!("api/v1/apps/{}/invoke/context", app_id))?;
        let client = self.http_client().client();

        let request = client
            .get(context_url)
            .header("Authorization", Self::authorization_value(token))
            .build()
            .map_err(flow_like_types::Error::from)?;

        // Each failure carries an `AuthorizationError`, so a caller can tell a hub it could
        // not reach (or that answered garbage) from one that refused the caller.
        let unavailable = |message: String| {
            flow_like_types::Error::new(AuthorizationError::Unavailable).context(message)
        };
        let resp = client
            .execute(request)
            .await
            .map_err(|e| unavailable(format!("execution context request failed: {}", e)))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| unavailable(format!("execution context body failed: {}", e)))?;

        if !status.is_success() {
            let kind = match status.as_u16() {
                401 => AuthorizationError::Expired,
                403 | 404 | 410 => AuthorizationError::Denied,
                408 | 429 | 500..=599 => AuthorizationError::Unavailable,
                _ => AuthorizationError::InvalidResponse,
            };
            return Err(flow_like_types::Error::new(kind).context(format!(
                "execution context failed: status={} body={}",
                status, body_text
            )));
        }

        flow_like_types::json::from_str(&body_text)
            .map_err(|e| unavailable(format!("JSON parse error: {}", e)))
    }

    /// Personal access tokens are sent verbatim; everything else is a bearer
    /// token and gets the scheme prefixed unless the caller already did.
    fn authorization_value(token: &str) -> String {
        if token.starts_with("pat_") || token.starts_with("Bearer ") {
            token.to_string()
        } else {
            format!("Bearer {}", token)
        }
    }

    pub async fn shared_credentials(&self, token: &str, app_id: &str) -> Result<SharedCredentials> {
        let presign_path = format!("api/v1/apps/{}/invoke/presign", app_id);

        let presign_url = self.construct_url(&presign_path)?;

        let auth_val = Self::authorization_value(token);

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

#[cfg(test)]
mod tests {
    use super::{HubRetry, hub_api_url, hub_error_message, hub_origin, send_hub_request};
    use flow_like_types::{
        reqwest::{Client, StatusCode},
        tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        },
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[test]
    fn hub_api_url_adds_the_api_base_once_and_encodes_segments() {
        for hub in ["example.com", "https://example.com/api/v1/"] {
            assert_eq!(
                hub_api_url(hub, true, &["apps", "app/1", "mail", "send"])
                    .unwrap()
                    .as_str(),
                "https://example.com/api/v1/apps/app%2F1/mail/send"
            );
        }
        assert_eq!(
            hub_api_url("http://api:8080", true, &["execution", "events"])
                .unwrap()
                .as_str(),
            "http://api:8080/api/v1/execution/events"
        );
        assert!(hub_api_url(" ", true, &["apps"]).is_err());
    }

    #[test]
    fn hub_error_message_prefers_the_public_message_and_stays_bounded() {
        assert_eq!(
            hub_error_message(r#"{"error":{"code":"BAD_REQUEST","message":"Recipient refused"}}"#),
            "Recipient refused"
        );
        assert_eq!(
            hub_error_message("  upstream timeout \n"),
            "upstream timeout"
        );
        let long = "é".repeat(400);
        let message = hub_error_message(&long);
        assert!(message.len() <= 512);
        assert!(long.starts_with(&message));
    }

    async fn serve(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        flow_like_types::tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await;
                counter.fetch_add(1, Ordering::SeqCst);
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (url, hits)
    }

    const TOO_MANY: &str = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
    const OK: &str = "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";

    fn only_429(retries: u32, max_wait: Duration) -> HubRetry {
        HubRetry {
            retryable: |status| status == StatusCode::TOO_MANY_REQUESTS,
            retries,
            max_wait,
        }
    }

    async fn get(url: &str, retry: HubRetry) -> (StatusCode, usize) {
        let client = Client::new();
        let builds = AtomicUsize::new(0);
        let (client_ref, builds_ref) = (&client, &builds);
        let response = send_hub_request(&client, retry, move || async move {
            builds_ref.fetch_add(1, Ordering::SeqCst);
            Ok::<_, flow_like_types::Error>(client_ref.get(url).build()?)
        })
        .await
        .unwrap();
        (response.status(), builds.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn send_hub_request_rebuilds_each_attempt_within_its_attempt_budget() {
        let (url, hits) = serve(vec![TOO_MANY, TOO_MANY, OK]).await;
        let (status, builds) = get(&url, only_429(3, Duration::from_secs(60))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((hits.load(Ordering::SeqCst), builds), (3, 3));

        let (url, hits) = serve(vec![TOO_MANY, TOO_MANY, OK]).await;
        let (status, builds) = get(&url, only_429(1, Duration::from_secs(60))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!((hits.load(Ordering::SeqCst), builds), (2, 2));
    }

    #[tokio::test]
    async fn send_hub_request_stops_when_the_wait_budget_is_spent() {
        let (url, hits) = serve(vec![
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 61\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        ])
        .await;
        let (status, _) = get(&url, only_429(3, Duration::from_secs(60))).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn hub_origin_attaches_the_scheme_the_profile_selected() {
        assert_eq!(
            hub_origin("api.flow-like.com", true).as_deref(),
            Some("https://api.flow-like.com")
        );
        assert_eq!(
            hub_origin("localhost:8080", false).as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(
            hub_origin("http://localhost:8080/", true).as_deref(),
            Some("http://localhost:8080")
        );
        assert_eq!(hub_origin("  ", true), None);
    }
}
