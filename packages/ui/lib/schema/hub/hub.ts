export interface IHub {
	app?: null | string;
	authentication?: null | IAuthentication;
	cdn?: null | string;
	contact: IContact;
	default_user_plan?: null | string;
	description: string;
	domain: string;
	environment: IEnvironment;
	features: IFeatures;
	hubs: string[];
	icon?: null | string;
	legal_notice: string;
	web?: null | string;
	lookup?: ILookup;
	max_users_prototype?: number | null;
	name: string;
	oauth_providers?: { [key: string]: IOAuthProviderConfig };
	privacy_policy: string;
	provider?: null | string;
	push_notifications?: IPushNotificationsConfig;
	region?: null | string;
	terms_of_service: string;
	signaling?: null | string[];
	/** Supported server-side event sinks */
	supported_sinks?: ISupportedSinks | null;
	thumbnail?: null | string;
	tiers: { [key: string]: IUserTier };
	[property: string]: any;
}

/**
 * Configuration for supported server-side event sinks.
 * When a hub is deployed, only sinks listed here will be available for server-side execution.
 */
export interface ISupportedSinks {
	/** HTTP/REST API endpoint sink */
	http?: boolean;
	/** Incoming webhook from external service */
	webhook?: boolean;
	/** Cron scheduled trigger */
	cron?: boolean;
	/** MQTT message broker */
	mqtt?: boolean;
	/** GitHub repository webhook */
	github?: boolean;
	/** RSS feed polling */
	rss?: boolean;
	/** Discord bot integration */
	discord?: boolean;
	/** Slack bot integration */
	slack?: boolean;
	/** Telegram bot integration */
	telegram?: boolean;
	/** Email/IMAP polling */
	email?: boolean;
	[property: string]: any;
}

export type PushNotificationProviderType =
	| "fcm"
	| "aws_sns"
	| "azure_notification_hubs";

export interface IFcmPushNotificationsConfig {
	project_id: string;
	service_account_json_env: string;
	[property: string]: any;
}

export interface IAwsSnsPushNotificationsConfig {
	android_platform_application_arn_env: string;
	ios_platform_application_arn_env: string;
	region_env?: string | null;
	[property: string]: any;
}

export interface IAzureNotificationHubsPushNotificationsConfig {
	namespace: string;
	hub_name: string;
	sas_key_name_env: string;
	sas_key_value_env: string;
	[property: string]: any;
}

export interface IPushNotificationsConfig {
	enabled?: boolean;
	allow_mobile?: boolean;
	allow_desktop?: boolean;
	provider?: PushNotificationProviderType | null;
	channel_id?: string | null;
	fcm?: IFcmPushNotificationsConfig | null;
	aws_sns?: IAwsSnsPushNotificationsConfig | null;
	azure_notification_hubs?: IAzureNotificationHubsPushNotificationsConfig | null;
	[property: string]: any;
}

/**
 * OAuth provider configuration from Hub.
 * This is the full configuration used to construct OAuth requests.
 */
export interface IOAuthProviderConfig {
	/** Display name shown to users */
	name: string;
	/** The resolved client ID (populated at runtime) */
	client_id?: string | null;
	/** OAuth authorization endpoint URL */
	auth_url: string;
	/** OAuth token endpoint URL */
	token_url: string;
	/** Base OAuth scopes */
	scopes?: string[];
	/** Whether PKCE is required */
	pkce_required?: boolean;
	/** Whether this provider requires the secret proxy for token exchange */
	requires_secret_proxy?: boolean;
	/** URL for token revocation */
	revoke_url?: string | null;
	/** URL for user info endpoint */
	userinfo_url?: string | null;
	/** Device authorization URL for device flow */
	device_auth_url?: string | null;
	/** Whether to use device flow */
	use_device_flow?: boolean;
	/** Whether to use implicit flow (response_type=token) instead of authorization code flow */
	use_implicit_flow?: boolean;
	/** Audience claim for token validation */
	audience?: string | null;
}

export interface IAuthentication {
	oauth2?: null | IOAuth2Config;
	openid?: null | IOpenIDConfig;
	variant: string;
	[property: string]: any;
}

export interface IOAuth2Config {
	authorization_endpoint: string;
	client_id: string;
	token_endpoint: string;
	[property: string]: any;
}

export interface IOpenIDConfig {
	authority?: null | string;
	client_id?: null | string;
	cognito?: null | ICognitoConfig;
	discovery_url?: null | string;
	user_info_url?: null | string;
	jwks_url: string;
	post_logout_redirect_uri?: null | string;
	proxy?: null | IOpenIDProxy;
	redirect_uri?: null | string;
	response_type?: null | string;
	scope?: null | string;
	[property: string]: any;
}

export interface ICognitoConfig {
	user_pool_id: string;
	[property: string]: any;
}

export interface IOpenIDProxy {
	authorize?: null | string;
	enabled: boolean;
	revoke?: null | string;
	token?: null | string;
	userinfo?: null | string;
	[property: string]: any;
}

export interface IContact {
	email: string;
	name: string;
	url: string;
	[property: string]: any;
}

export enum IEnvironment {
	Development = "Development",
	Production = "Production",
	Staging = "Staging",
}

export interface IFeatures {
	admin_interface: boolean;
	ai_act: boolean;
	flow_hosting: boolean;
	governance: boolean;
	model_hosting: boolean;
	premium: boolean;
	unauthorized_read: boolean;
	[property: string]: any;
}

export interface ILookup {
	additional_information: boolean;
	avatar: boolean;
	created_at: boolean;
	description: boolean;
	email: boolean;
	name: boolean;
	preferred_username: boolean;
	username: boolean;
	[property: string]: any;
}

export interface IUserTier {
	execution_tier: string;
	llm_tiers: string[];
	max_llm_cost: number;
	max_llm_calls?: number;
	max_non_visible_projects: number;
	max_remote_executions: number;
	max_total_size: number;
	product_id?: string | null;
	[property: string]: any;
}
