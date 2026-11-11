export interface IHub {
    alerting?: null | IAlertingObject;
    app?:      null | string;
    /**
     * IAudit trail configuration
     */
    audit?:          IAudit;
    authentication?: null | IAuthenticationObject;
    cdn?:            null | string;
    contact:         IContact;
    /**
     * Upgrade / conversion experience configuration
     */
    conversion?:        IConversion;
    default_user_plan?: null | string;
    description:        string;
    domain:             string;
    environment:        IEnvironment;
    features:           IFeatures;
    /**
     * Fork-an-app feature configuration
     */
    forking?:             IForking;
    hubs:                 string[];
    icon?:                null | string;
    legal_notice:         string;
    lookup?:              ILookup;
    mail?:                null | IMailObject;
    max_users_prototype?: number | null;
    name:                 string;
    /**
     * OAuth provider configurations
     */
    oauth_providers?: { [key: string]: IOauthProviderValue };
    payments?:        IPayments;
    privacy_policy:   string;
    provider?:        null | string;
    /**
     * Push notification provider configuration
     */
    push_notifications?: IPushNotifications;
    /**
     * IRealtime transport configuration. Signaling remains configured separately.
     */
    realtime?:   IRealtime;
    region?:     null | string;
    secure?:     boolean;
    signaling?:  string[] | null;
    standalone?: IStandalone;
    /**
     * Supported server-side event sinks (e.g., discord, telegram, cron, http)
     * If None, defaults to basic sinks like http, webhook, cron
     */
    supported_sinks?: null | ISupportedSinksObject;
    terms_of_service: string;
    thumbnail?:       null | string;
    tiers:            { [key: string]: ITierValue };
    /**
     * WASM registry configuration
     */
    wasm_registry_config?: IWASMRegistryConfig;
    web?:                  null | string;
    /**
     * Where apps' Flow-Like storage files are served. A widget can only be
     * granted `{origin}{path_prefix}{appId}/` on these origins, never the
     * whole origin.
     */
    widget_storage?: IWidgetStorageElement[];
    [property: string]: any;
}

export interface IAlertingObject {
    mail: string;
    [property: string]: any;
}

/**
 * IAudit trail configuration
 */
export interface IAudit {
    /**
     * Master switch. When false, no audit entries are recorded.
     */
    enabled?: boolean;
    /**
     * Which action families are recorded. Defaults to `standard`.
     */
    level?: string;
    /**
     * Record execution lifecycle transitions at any level.
     * The `verbose` level records them regardless of this switch.
     */
    log_executions?: boolean;
    /**
     * Whether to capture client IP addresses in audit entries (GDPR consideration)
     */
    log_ip?: boolean;
    /**
     * Refuse to run without keys: the API without an entry key, the audit worker
     * without an audit signing key.
     */
    require_signing?: boolean;
    /**
     * How long each part of the trail is kept, and when the audit worker seals.
     */
    retention?: IRetention;
    /**
     * Reverse proxies under the deployment's control that append to
     * `X-Forwarded-For`. The recorded client IP is taken that many entries from
     * the right, which a client cannot forge. Unset records the leftmost entry,
     * which the client chooses.
     */
    trusted_proxy_hops?: number | null;
    [property: string]: any;
}

/**
 * How long each part of the trail is kept, and when the audit worker seals.
 *
 * Retention, sealing and signing cadence. Evidence is pruned only once its month is
 * archived to the audit bucket, so a deployment without `AUDIT_BUCKET` keeps every
 * evidence record; activity records are deleted after their window.
 */
export interface IRetention {
    /**
     * Days records of verbose-only actions stay in the database. They are never
     * archived.
     */
    activity_days?: number;
    /**
     * Days after a month closes before it is archived.
     */
    archive_grace_days?: number;
    /**
     * Informational horizon written into archive manifests: the end of this many
     * calendar years after the month. The bucket's retention lock enforces it.
     */
    archive_years_after_year_end?: number;
    /**
     * Days record details are kept. Unset keeps them as long as the record.
     */
    details_days?: number | null;
    /**
     * Sign an epoch once the oldest unanchored seal is this old, or earlier when 20,000
     * seals wait. Each epoch is one request to the key service, so this sets its cost:
     * 300 s means at most 12 routine signatures an hour.
     */
    epoch_interval_seconds?: number;
    /**
     * Days security and content records stay in the database after their month
     * closes. Older months are read from the monthly archive.
     */
    evidence_hot_days?: number;
    /**
     * Minimum days for execution records of apps assessed as high-risk AI systems
     * (EU AI Act art. 19 and 26(6)).
     */
    high_risk_activity_days?: number;
    /**
     * Days a recorded client IP is kept before it is removed from its record.
     */
    ip_days?: number;
    /**
     * Largest seal the worker writes in one transaction.
     */
    max_records_per_seal?: number;
    /**
     * Log an alert when the oldest pending record is older than this.
     */
    pending_alert_seconds?: number;
    /**
     * Seal a chain once this many records are pending.
     */
    seal_after_records?: number;
    /**
     * Seal a chain once its oldest pending record is this old.
     */
    seal_after_seconds?: number;
    [property: string]: any;
}

export interface IAuthenticationObject {
    oauth2?: null | IOauth2Object;
    openid?: null | IOpenidObject;
    variant: string;
    [property: string]: any;
}

export interface IOauth2Object {
    authorization_endpoint: string;
    client_id:              string;
    token_endpoint:         string;
    [property: string]: any;
}

export interface IOpenidObject {
    /**
     * Exact token audience accepted by the API. Defaults to `client_id`,
     * which is the correct target for OIDC ID tokens and Cognito tokens.
     */
    audience?:      null | string;
    authority?:     null | string;
    client_id?:     null | string;
    cognito?:       null | ICognitoObject;
    discovery_url?: null | string;
    /**
     * Exact token issuer (`iss`) accepted by the API. When omitted for an
     * existing deployment, `authority` remains the compatibility fallback.
     */
    issuer?:                   null | string;
    jwks_url:                  string;
    post_logout_redirect_uri?: null | string;
    proxy?:                    null | IProxyObject;
    redirect_uri?:             null | string;
    response_type?:            null | string;
    scope?:                    null | string;
    user_info_url?:            null | string;
    [property: string]: any;
}

export interface ICognitoObject {
    user_pool_id: string;
    [property: string]: any;
}

export interface IProxyObject {
    authorize?: null | string;
    enabled:    boolean;
    revoke?:    null | string;
    token?:     null | string;
    userinfo?:  null | string;
    [property: string]: any;
}

export interface IContact {
    email: string;
    name:  string;
    url:   string;
    [property: string]: any;
}

/**
 * Upgrade / conversion experience configuration
 */
export interface IConversion {
    /**
     * IContact used in enterprise mode and for the enterprise tier CTA;
     * falls back to the hub contact when unset
     */
    contact?: null | IContact;
    /**
     * Custom message shown on the enterprise contact card
     */
    contact_message?: null | string;
    /**
     * When false, plan-limit errors surface as plain messages without the
     * upgrade dialog
     */
    enabled?: boolean;
    /**
     * Headline override for the upgrade dialog
     */
    headline?: null | string;
    mode?:     IMode;
    /**
     * Supporting line under the headline
     */
    subheadline?: null | string;
    /**
     * Per-tier marketing metadata keyed by tier id (FREE, PREMIUM, ...)
     */
    tier_display?: { [key: string]: ITierDisplayValue };
    [property: string]: any;
}

/**
 * Decides what users see when they hit a plan limit: a self-service upgrade
 * flow (consumer) or a "contact us" card (enterprise deployments).
 */
export enum IMode {
    Consumer = "consumer",
    Enterprise = "enterprise",
}

/**
 * Marketing metadata for a tier, keyed by tier id in `ConversionConfig`.
 */
export interface ITierDisplayValue {
    annual_price_cents?: number | null;
    /**
     * Badge text overriding the default highlight label
     */
    badge?:    null | string;
    currency?: null | string;
    /**
     * Name shown instead of the raw tier key (e.g. "Starter" for FREE)
     */
    display_name?: null | string;
    /**
     * Curated feature bullets shown in addition to the derived limit facts
     */
    features?: string[];
    /**
     * Marks the recommended tier — gets the "Most popular" treatment
     */
    highlight?:           boolean;
    monthly_price_cents?: number | null;
    /**
     * One-line value proposition under the tier name
     */
    tagline?: null | string;
    [property: string]: any;
}

export enum IEnvironment {
    Development = "Development",
    Production = "Production",
    Staging = "Staging",
}

export interface IFeatures {
    admin_interface:               boolean;
    ai_act:                        boolean;
    app_package_linking?:          boolean;
    flow_hosting:                  boolean;
    governance:                    boolean;
    model_hosting:                 boolean;
    premium:                       boolean;
    telemetry?:                    boolean;
    unauthorized_read:             boolean;
    wasm_package_user_management?: boolean;
    wasm_registry?:                boolean;
    wasm_server_compilation?:      boolean;
    [property: string]: any;
}

/**
 * Fork-an-app feature configuration
 *
 * Fork-an-app feature config. Controls quotas and the unauthenticated-fork
 * path. Defaults are conservative: feature on, 1 GB / 10k file cap, no
 * anonymous forking.
 */
export interface IForking {
    /**
     * Whether anonymous (unauthenticated) callers may fork a public+free app
     * to an offline (desktop) destination
     */
    allow_unauthenticated_to_offline?: boolean;
    /**
     * Global kill switch — when false, every fork endpoint refuses
     */
    enabled?: boolean;
    /**
     * Hard cap on the number of objects copied in a single fork
     */
    max_file_count?: number;
    /**
     * Hard cap on the total bytes copied in a single fork
     */
    max_size_bytes?: number;
    [property: string]: any;
}

export interface ILookup {
    additional_information: boolean;
    avatar:                 boolean;
    created_at:             boolean;
    description:            boolean;
    email:                  boolean;
    name:                   boolean;
    preferred_username?:    boolean;
    username:               boolean;
    [property: string]: any;
}

export interface IMailObject {
    from_email: string;
    from_name:  string;
    provider:   IProvider;
    sendgrid?:  null | ISendgridObject;
    smtp?:      null | ISMTPObject;
    [property: string]: any;
}

export enum IProvider {
    AzureCommunicationServices = "azure_communication_services",
    SMTP = "smtp",
    Sendgrid = "sendgrid",
    Ses = "ses",
}

export interface ISendgridObject {
    api_key_env: string;
    [property: string]: any;
}

export interface ISMTPObject {
    host_env:     string;
    password_env: string;
    port_env:     string;
    username_env: string;
    [property: string]: any;
}

/**
 * OAuth provider configuration from the config file.
 * This is used to configure OAuth providers centrally.
 * The client_secret is resolved from environment variables at build time for providers that
 * need it.
 */
export interface IOauthProviderValue {
    /**
     * Hosts a bot's connector may send this provider's token to (exact names or `*.suffix`).
     * Empty uses the built-in list for the provider id.
     */
    api_hosts?: string[];
    /**
     * Optional: Audience claim for token validation
     */
    audience?: null | string;
    /**
     * OAuth authorization endpoint URL
     */
    auth_url: string;
    /**
     * The client ID (public, not secret)
     */
    client_id?: string;
    /**
     * The resolved client secret (populated at build time from the env var)
     */
    client_secret?: null | string;
    /**
     * IEnvironment variable name containing the client secret (resolved at build time)
     * If null, no secret is needed (PKCE-based flow)
     */
    client_secret_env?: null | string;
    /**
     * Optional: Device authorization URL for device flow
     */
    device_auth_url?: null | string;
    /**
     * Optional: the provider's MCP endpoint, which may receive the provider's token.
     */
    mcp_url?: null | string;
    /**
     * Display name shown to users
     */
    name: string;
    /**
     * Whether PKCE is required
     */
    pkce_required?: boolean;
    /**
     * Whether this provider requires the secret proxy for token exchange
     * If true, token exchange requests go through the API server which adds the secret
     */
    requires_secret_proxy?: boolean;
    /**
     * Optional: URL for token revocation
     */
    revoke_url?: null | string;
    /**
     * Base OAuth scopes (node-specific scopes will be added by the frontend)
     */
    scopes?: string[];
    /**
     * OAuth token endpoint URL
     */
    token_url: string;
    /**
     * Whether to use device flow
     */
    use_device_flow?:   boolean;
    use_implicit_flow?: boolean;
    /**
     * Optional: URL for user info endpoint
     */
    userinfo_url?: null | string;
    [property: string]: any;
}

export interface IPayments {
    countries?:                        string[];
    currencies?:                       string[];
    frontend_url?:                     null | string;
    legacy_checkout_until?:            number | null;
    legal_texts?:                      ILegalTextElement[];
    live_approved?:                    boolean;
    livemode?:                         boolean;
    marketplace_enabled?:              boolean;
    marketplace_fee_basis?:            IMarketplaceFeeBasis;
    marketplace_fee_bps?:              number;
    marketplace_method_configuration?: null | string;
    marketplace_min_amount?:           number;
    marketplace_payment_methods?:      string[];
    marketplace_tax_mode?:             IMarketplaceTaxMode;
    max_payment_amount?:               number;
    new_seller_daily_cap?:             number;
    node_fee_bps?:                     number;
    node_method_configuration?:        null | string;
    node_payment_methods?:             string[];
    node_payments_enabled?:            boolean;
    node_tax_mode?:                    IMarketplaceTaxMode;
    onboarding_enabled?:               boolean;
    owner_terms_version?:              null | string;
    platform_account_id?:              null | string;
    product_tax_code?:                 null | string;
    purchase_terms_version?:           null | string;
    purchase_waiver_version?:          null | string;
    seller_allowlist?:                 string[];
    seller_terms_version?:             null | string;
    servicing_enabled?:                boolean;
    withdrawal_days?:                  number;
    [property: string]: any;
}

export interface ILegalTextElement {
    hash?:   null | string;
    kind:    string;
    locale:  string;
    text?:   string;
    url?:    null | string;
    version: string;
    [property: string]: any;
}

export enum IMarketplaceFeeBasis {
    Gross = "gross",
    NetOfTax = "net_of_tax",
}

export enum IMarketplaceTaxMode {
    PlatformSupplier = "platform_supplier",
    SellerSupplier = "seller_supplier",
    Unconfigured = "unconfigured",
}

/**
 * Push notification provider configuration
 */
export interface IPushNotifications {
    allow_desktop?:           boolean;
    allow_mobile?:            boolean;
    aws_sns?:                 null | IAwsSnsObject;
    azure_notification_hubs?: null | IAzureNotificationHubsObject;
    channel_id?:              null | string;
    enabled?:                 boolean;
    fcm?:                     null | IFcmObject;
    provider?:                IProviderEnum | null;
    [property: string]: any;
}

export interface IAwsSnsObject {
    android_platform_application_arn_env: string;
    ios_platform_application_arn_env:     string;
    region_env?:                          null | string;
    [property: string]: any;
}

export interface IAzureNotificationHubsObject {
    hub_name:          string;
    namespace:         string;
    sas_key_name_env:  string;
    sas_key_value_env: string;
    [property: string]: any;
}

export interface IFcmObject {
    project_id:               string;
    service_account_json_env: string;
    [property: string]: any;
}

export enum IProviderEnum {
    AwsSns = "aws_sns",
    AzureNotificationHubs = "azure_notification_hubs",
    Fcm = "fcm",
}

/**
 * IRealtime transport configuration. Signaling remains configured separately.
 */
export interface IRealtime {
    /**
     * Omit this field to retain the WebRTC library's built-in ICE defaults.
     */
    ice?: null | IIceObject;
    [property: string]: any;
}

/**
 * Selects the service that issues short-lived STUN and TURN configuration for
 * realtime clients. IProvider secrets are resolved by the API from references
 * in the secret store and never belong in the hub JSON itself.
 */
export interface IIceObject {
    provider: string;
    /**
     * Lifetime of each client credential. Cloudflare accepts at most 48 hours.
     */
    ttl_seconds?: number;
    /**
     * Secret-store reference containing the bearer key returned with the TURN key.
     */
    turn_key_api_token_secret_ref: string;
    /**
     * Secret-store reference containing the Cloudflare TURN key identifier.
     */
    turn_key_id_secret_ref: string;
    [property: string]: any;
}

/**
 * Device enrollment limits. Enabling enrollment requires the device registry migration.
 */
export interface IStandalone {
    /**
     * Canonical public API base, including `/api/v1`. Defaults to the hub origin.
     */
    api_base_url?:                     null | string;
    enabled?:                          boolean;
    enrollment_ttl_seconds?:           number;
    max_devices_per_user?:             number;
    max_pending_enrollments_per_user?: number;
    /**
     * Release authority is configured independently of downloaded release manifests.
     */
    release_trust?: null | IReleaseTrustObject;
    /**
     * Encrypted retained telemetry limits per account tier. Missing tiers cannot store history.
     */
    telemetry_tiers?: { [key: string]: ITelemetryTierValue };
    [property: string]: any;
}

export interface IReleaseTrustObject {
    manifest_url:      string;
    minimum_sequence?: number;
    /**
     * Canonical base64url Ed25519 public keys, 32 bytes each.
     */
    public_keys: string[];
    [property: string]: any;
}

export interface ITelemetryTierValue {
    max_bytes?:         number;
    retention_seconds?: number;
    [property: string]: any;
}

/**
 * Configuration for supported server-side event sinks.
 * When a hub is deployed, only sinks listed here will be available for server-side
 * execution.
 * Desktop availability is defined by each sink type.
 */
export interface ISupportedSinksObject {
    /**
     * Cron scheduled trigger
     */
    cron?: boolean;
    /**
     * Discord bot integration
     */
    discord?: boolean;
    /**
     * Email/IMAP polling
     */
    email?: boolean;
    /**
     * GitHub repository webhook
     */
    github?: boolean;
    /**
     * HTTP/REST API endpoint sink
     */
    http?: boolean;
    /**
     * Email received at an address issued by the server
     */
    inbound_email?: boolean;
    /**
     * MQTT message broker
     */
    mqtt?: boolean;
    /**
     * RSS feed polling
     */
    rss?: boolean;
    /**
     * Slack bot integration
     */
    slack?: boolean;
    /**
     * Microsoft Teams bot webhook
     */
    teams?: boolean;
    /**
     * Telegram bot integration
     */
    telegram?: boolean;
    /**
     * Incoming webhook from external service
     */
    webhook?: boolean;
    [property: string]: any;
}

export interface ITierValue {
    execution_tier: string;
    llm_tiers:      string[];
    /**
     * Monthly hosted AI allowance in EUR millionths, including serving costs.
     */
    max_ai_cost_micros?:        number;
    max_concurrent_executions?: number;
    max_llm_calls?:             number | null;
    max_llm_cost:               number;
    max_non_visible_projects:   number;
    max_remote_executions:      number;
    /**
     * Monthly cloud workflow runtime in milliseconds; negative means unlimited.
     */
    max_runtime_ms?: number;
    max_total_size:  number;
    product_id?:     null | string;
    [property: string]: any;
}

/**
 * WASM registry configuration
 */
export interface IWASMRegistryConfig {
    /**
     * What happens to app packages when the member who added them leaves the app
     */
    on_member_leave?: string;
    [property: string]: any;
}

export interface IWidgetStorageElement {
    /**
     * `scheme://host` the content bucket is served from
     */
    origin: string;
    /**
     * Path before the app id, starting and ending with `/`
     */
    path_prefix: string;
    [property: string]: any;
}
