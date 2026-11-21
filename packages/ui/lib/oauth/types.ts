/**
 * Full OAuth provider configuration used for OAuth operations.
 * This is constructed by merging Hub config with node provider references.
 */
export interface IOAuthProvider {
	id: string;
	name: string;
	auth_url: string;
	token_url: string;
	client_id: string;
	/** Client secret for providers that require it (e.g., Notion). Leave empty for PKCE-based flows. */
	client_secret?: string;
	scopes: string[];
	pkce_required: boolean;
	/**
	 * Whether this provider requires the secret proxy for token exchange.
	 * If true, token exchange goes through the Hub, which adds the client secret.
	 * Refreshable connections use Hub-owned refresh tokens regardless of this flag.
	 */
	requires_secret_proxy: boolean;
	revoke_url?: string;
	userinfo_url?: string;
	oidc_discovery_url?: string;
	jwks_url?: string;
	audience?: string;
	/** Device authorization endpoint URL for device flow (RFC 8628) */
	device_auth_url?: string;
	/** Whether to use device flow instead of standard authorization code flow */
	use_device_flow: boolean;
	/** Whether to use implicit flow (returns token directly in URL fragment, no code exchange) */
	use_implicit_flow?: boolean;
	/** Additional scopes merged from nodes that require this provider */
	merged_scopes?: string[];
}

/**
 * Device flow authorization response from the device authorization endpoint
 */
export interface IDeviceAuthResponse {
	device_code: string;
	user_code: string;
	verification_uri: string;
	verification_uri_complete?: string;
	expires_in: number;
	interval: number;
}

/**
 * OAuth token stored/passed to backend.
 * This matches the Rust OAuthToken struct.
 */
export interface IOAuthToken {
	hub_origin?: string;
	hub_subject?: string;
	access_token: string;
	refresh_token?: string;
	expires_at?: number;
	token_type?: string;
}

/**
 * Extended OAuth token with additional metadata for storage.
 */
export interface IStoredOAuthToken extends IOAuthToken {
	providerId: string;
	binding?: IOAuthBinding;
	scopes: string[];
	storedAt: number;
	userInfo?: {
		sub?: string;
		email?: string;
		name?: string;
		picture?: string;
	};
}

export interface IOAuthBinding {
	sessionKey: string;
	providerKey: string;
	/** The Hub that proxies this provider's secret; it owns the grant only when `local` is unset. */
	apiBaseUrl: string;
	local?: true;
}

/**
 * Result of checking OAuth tokens for providers
 */
export interface IOAuthTokenCheckResult {
	tokens: Record<string, IOAuthToken>;
	missingProviders: IOAuthProvider[];
}

/**
 * Interface for OAuth token storage implementations.
 * Desktop app uses Dexie, web app might use different storage.
 */
export interface IOAuthTokenStore {
	getToken(
		providerId: string,
		provider?: IOAuthProvider,
	): Promise<IStoredOAuthToken | undefined>;
	setToken(token: IStoredOAuthToken): Promise<void>;
	deleteToken(providerId: string): Promise<void>;
	getAllTokens(): Promise<IStoredOAuthToken[]>;
	isExpired(token: IStoredOAuthToken, bufferMs?: number): boolean;
	getBinding?(provider: IOAuthProvider): Promise<IOAuthBinding>;
	isCurrentBinding?(binding: IOAuthBinding): Promise<boolean>;
	replaceToken?(
		expected: IStoredOAuthToken,
		replacement?: IStoredOAuthToken,
	): Promise<boolean>;
	withRefreshLock?<T>(
		token: IStoredOAuthToken,
		operation: () => Promise<T>,
	): Promise<T>;
}

/**
 * Pending OAuth authorization request
 */
export interface IOAuthPendingAuth {
	state: string;
	binding?: IOAuthBinding;
	providerId: string;
	codeVerifier: string;
	redirectUri: string;
	scopes: string[];
	initiatedAt: number;
	appId?: string;
	boardId?: string;
	/** Full provider configuration stored for callback handling after page reload */
	provider?: IOAuthProvider;
	/** API base URL for OAuth proxy (required for providers with requires_secret_proxy=true) */
	apiBaseUrl?: string;
}

/**
 * Extended token store with pending auth support
 */
export interface IOAuthTokenStoreWithPending extends IOAuthTokenStore {
	setPendingAuth(pending: IOAuthPendingAuth): Promise<void>;
	getPendingAuth(state: string): Promise<IOAuthPendingAuth | undefined>;
	consumePendingAuth(state: string): Promise<IOAuthPendingAuth | undefined>;
	cleanupPendingAuth(): Promise<void>;
}

/**
 * Platform-specific OAuth runtime operations.
 * Abstraction for platform-specific APIs (Tauri, browser, etc.)
 */
export interface IOAuthRuntime {
	/** Open a URL in the system browser or webview */
	openUrl(url: string): Promise<void>;
	/** Make an HTTP POST request */
	httpPost(
		url: string,
		body: string,
		headers?: Record<string, string>,
		binding?: IOAuthBinding,
	): Promise<{
		ok: boolean;
		status: number;
		json: () => Promise<unknown>;
		text: () => Promise<string>;
	}>;
	/** Make an HTTP GET request */
	httpGet(
		url: string,
		headers?: Record<string, string>,
		binding?: IOAuthBinding,
	): Promise<{
		ok: boolean;
		status: number;
		json: () => Promise<unknown>;
		text: () => Promise<string>;
	}>;
}

/**
 * OAuth callback data received from the authorization server
 */
export interface IOAuthCallbackData {
	url: string;
	code: string | null;
	state: string | null;
	id_token: string | null;
	access_token: string | null;
	token_type: string | null;
	expires_in: string | null;
	scope: string | null;
	error: string | null;
	error_description: string | null;
}
