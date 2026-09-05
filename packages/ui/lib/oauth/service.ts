import type {
	IDeviceAuthResponse,
	IOAuthProvider,
	IOAuthRuntime,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "./types";

function generateRandomString(length: number): string {
	const array = new Uint8Array(length);
	crypto.getRandomValues(array);
	return Array.from(array, (b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Platform targets for OAuth callback routing.
 * These map to hardcoded URLs on the callback handler for security.
 */
export type OAuthPlatform = "desktop" | "web-dev" | "web-prod";

/**
 * Encodes platform info into the OAuth state parameter.
 * Format: platform_randomState
 * Uses predefined platform flags that map to hardcoded URLs on the callback handler.
 */
function encodeStateWithPlatform(
	randomState: string,
	platform: OAuthPlatform,
): string {
	return `${platform}_${randomState}`;
}

/**
 * Decodes platform info from the OAuth state parameter.
 * Returns the platform flag and random state.
 */
export function decodeStateWithPlatform(state: string): {
	platform: OAuthPlatform;
	randomState: string;
} {
	const platforms: OAuthPlatform[] = ["desktop", "web-dev", "web-prod"];
	for (const platform of platforms) {
		if (state.startsWith(`${platform}_`)) {
			return {
				platform,
				randomState: state.substring(platform.length + 1),
			};
		}
	}
	// Legacy state without platform prefix - assume desktop
	return { platform: "desktop", randomState: state };
}

function generateCodeVerifier(): string {
	return generateRandomString(64);
}

async function generateCodeChallenge(verifier: string): Promise<string> {
	const encoder = new TextEncoder();
	const data = encoder.encode(verifier);
	const digest = await crypto.subtle.digest("SHA-256", data);
	return btoa(String.fromCharCode(...new Uint8Array(digest)))
		.replace(/\+/g, "-")
		.replace(/\//g, "_")
		.replace(/=+$/, "");
}

export interface OAuthServiceConfig {
	runtime: IOAuthRuntime;
	tokenStore: IOAuthTokenStoreWithPending;
	redirectUri?: string;
	/**
	 * Function to get the API base URL for the OAuth proxy.
	 * Required for providers with requires_secret_proxy=true.
	 * Should return the base URL like "https://api.example.com"
	 */
	getApiBaseUrl?: () => Promise<string | null>;
	/**
	 * Platform identifier for OAuth callback routing.
	 * This value is encoded into OAuth state and interpreted by the website callback
	 * page to perform the second-hop redirect after provider callback is received.
	 * - "desktop": Website callback forwards to the native app deep link route
	 * - "web-dev": Website callback forwards to localhost web callback
	 * - "web-prod": Website callback forwards to production web callback
	 */
	platform?: OAuthPlatform;
}

export function createOAuthService(config: OAuthServiceConfig) {
	const { runtime, tokenStore, getApiBaseUrl, platform = "desktop" } = config;
	// Always use website callback as OAuth redirect URI.
	// The website callback script decides the final destination based on state.
	const redirectUri =
		config.redirectUri ?? "https://flow-like.com/thirdparty/callback";

	function shouldProxyProviderRequests(provider: IOAuthProvider): boolean {
		return provider.requires_secret_proxy;
	}

	async function resolveApiBaseUrl(
		provider: IOAuthProvider,
		overrideApiBaseUrl?: string,
	): Promise<string> {
		const rawApiBaseUrl =
			overrideApiBaseUrl ?? (getApiBaseUrl ? await getApiBaseUrl() : null);
		if (!rawApiBaseUrl) {
			throw new Error(
				`Provider ${provider.id} requires OAuth proxying but no API base URL is available`,
			);
		}

		return rawApiBaseUrl.endsWith("/")
			? rawApiBaseUrl.slice(0, -1)
			: rawApiBaseUrl;
	}

	return {
		async startAuthorization(
			provider: IOAuthProvider,
			options?: {
				appId?: string;
				boardId?: string;
				additionalScopes?: string[];
			},
		): Promise<{ state: string; authUrl: string }> {
			// Validate client_id is present
			if (!provider.client_id || provider.client_id.trim() === "") {
				throw new Error(
					`OAuth client_id is not configured for ${provider.name}. ` +
						`Please set the environment variable or provide it via input pin.`,
				);
			}

			await tokenStore.cleanupPendingAuth();

			const randomState = generateRandomString(32);
			// Encode platform info into state for callback routing
			const state = encodeStateWithPlatform(randomState, platform);
			const codeVerifier = generateCodeVerifier();
			const codeChallenge = await generateCodeChallenge(codeVerifier);

			// Use merged_scopes if available (includes node-required scopes), otherwise fall back to base scopes
			const scopes = [...(provider.merged_scopes ?? provider.scopes)];
			if (options?.additionalScopes) {
				for (const scope of options.additionalScopes) {
					if (!scopes.includes(scope)) {
						scopes.push(scope);
					}
				}
			}

			await tokenStore.setPendingAuth({
				state,
				providerId: provider.id,
				codeVerifier,
				redirectUri,
				scopes,
				initiatedAt: Date.now(),
				appId: options?.appId,
				boardId: options?.boardId,
				provider, // Store full provider for callback handling
				apiBaseUrl: getApiBaseUrl
					? ((await getApiBaseUrl()) ?? undefined)
					: undefined,
			});

			// Use implicit flow (response_type=token) if configured, otherwise authorization code flow
			console.log(
				"[OAuth] Provider use_implicit_flow value:",
				provider.use_implicit_flow,
				"type:",
				typeof provider.use_implicit_flow,
			);
			const responseType = provider.use_implicit_flow ? "token" : "code";
			console.log("[OAuth] Using responseType:", responseType);

			const params = new URLSearchParams({
				client_id: provider.client_id,
				redirect_uri: redirectUri,
				response_type: responseType,
				scope: scopes.join(" "),
				state,
			});

			// Add PKCE parameters for providers that require it (not used in implicit flow)
			if (provider.pkce_required && !provider.use_implicit_flow) {
				params.set("code_challenge", codeChallenge);
				params.set("code_challenge_method", "S256");
			}

			// For providers without PKCE (like Notion), add owner=user
			// and skip access_type/prompt which are Google-specific
			if (!provider.pkce_required && !provider.use_implicit_flow) {
				params.set("owner", "user");
			} else if (provider.pkce_required && !provider.use_implicit_flow) {
				// Authorization code flow with PKCE: request offline access for refresh token
				params.set("access_type", "offline");
				params.set("prompt", "consent");
			}
			// Implicit flow: no special parameters needed (no refresh tokens available)

			console.log("[OAuth] Starting authorization:", {
				providerId: provider.id,
				providerName: provider.name,
				clientId: provider.client_id,
				hasClientSecret: !!provider.client_secret,
				pkceRequired: provider.pkce_required,
				requiresSecretProxy: provider.requires_secret_proxy,
				useImplicitFlow: provider.use_implicit_flow,
				responseType,
				scopes,
			});

			const authUrl = `${provider.auth_url}?${params.toString()}`;
			await runtime.openUrl(authUrl);

			return { state, authUrl };
		},

		async handleCallback(
			callbackUrl: string,
			provider: IOAuthProvider,
		): Promise<IStoredOAuthToken> {
			const url = new URL(callbackUrl);
			const code = url.searchParams.get("code");
			const state = url.searchParams.get("state");
			const error = url.searchParams.get("error");
			const errorDescription = url.searchParams.get("error_description");

			console.log("[OAuth] handleCallback called:", {
				providerId: provider.id,
				providerName: provider.name,
				clientId: provider.client_id,
				hasClientSecret: !!provider.client_secret,
				pkceRequired: provider.pkce_required,
				requiresSecretProxy: provider.requires_secret_proxy,
				hasCode: !!code,
				error,
			});

			if (error) {
				throw new Error(
					`OAuth error: ${error}${errorDescription ? ` - ${errorDescription}` : ""}`,
				);
			}

			if (!code || !state) {
				throw new Error("Missing code or state in OAuth callback");
			}

			const pendingAuth = await tokenStore.consumePendingAuth(state);
			if (!pendingAuth) {
				throw new Error("Invalid or expired OAuth state");
			}

			console.log("[OAuth] pendingAuth found:", {
				providerId: pendingAuth.providerId,
				hasProvider: !!pendingAuth.provider,
				pendingProviderClientId: pendingAuth.provider?.client_id,
				hasCodeVerifier: !!pendingAuth.codeVerifier,
			});

			if (pendingAuth.providerId !== provider.id) {
				throw new Error("Provider mismatch in OAuth callback");
			}

			const tokenResponse = await this.exchangeCodeForTokens(
				provider,
				code,
				pendingAuth.codeVerifier,
				pendingAuth.redirectUri,
				pendingAuth.apiBaseUrl,
			);

			let userInfo: IStoredOAuthToken["userInfo"];
			if (provider.userinfo_url && tokenResponse.access_token) {
				try {
					userInfo = await this.fetchUserInfo(
						provider,
						tokenResponse.access_token,
						pendingAuth.apiBaseUrl,
					);
				} catch {
					console.warn("Failed to fetch OAuth user info");
				}
			}

			const token: IStoredOAuthToken = {
				providerId: provider.id,
				access_token: tokenResponse.access_token,
				refresh_token: tokenResponse.refresh_token,
				expires_at: tokenResponse.expires_in
					? Date.now() + tokenResponse.expires_in * 1000
					: undefined,
				token_type: tokenResponse.token_type ?? "Bearer",
				scopes: pendingAuth.scopes,
				storedAt: Date.now(),
				userInfo,
			};

			await tokenStore.setToken(token);
			return token;
		},

		async handleImplicitCallback(
			pendingAuth: {
				providerId: string;
				scopes: string[];
				state: string;
				apiBaseUrl?: string;
			},
			provider: IOAuthProvider,
			tokenData: {
				access_token: string;
				id_token?: string;
				token_type?: string;
				expires_in?: number;
				scope?: string;
			},
		): Promise<IStoredOAuthToken> {
			await tokenStore.consumePendingAuth(pendingAuth.state);

			let userInfo: IStoredOAuthToken["userInfo"];
			if (provider.userinfo_url && tokenData.access_token) {
				try {
					userInfo = await this.fetchUserInfo(
						provider,
						tokenData.access_token,
						pendingAuth.apiBaseUrl,
					);
				} catch {
					console.warn("Failed to fetch OAuth user info");
				}
			}

			const scopes = tokenData.scope
				? tokenData.scope.split(" ")
				: pendingAuth.scopes;

			const token: IStoredOAuthToken = {
				providerId: provider.id,
				access_token: tokenData.access_token,
				refresh_token: undefined,
				expires_at: tokenData.expires_in
					? Date.now() + tokenData.expires_in * 1000
					: undefined,
				token_type: tokenData.token_type ?? "Bearer",
				scopes,
				storedAt: Date.now(),
				userInfo,
			};

			await tokenStore.setToken(token);
			return token;
		},

		async exchangeCodeForTokens(
			provider: IOAuthProvider,
			code: string,
			codeVerifier: string,
			callbackRedirectUri: string,
			overrideApiBaseUrl?: string,
		): Promise<{
			access_token: string;
			refresh_token?: string;
			expires_in?: number;
			token_type?: string;
			id_token?: string;
			workspace_id?: string;
			workspace_name?: string;
			workspace_icon?: string;
			bot_id?: string;
			owner?: unknown;
		}> {
			console.log("[OAuth] exchangeCodeForTokens called:", {
				providerId: provider.id,
				hasCode: !!code,
				hasCodeVerifier: !!codeVerifier,
				redirectUri: callbackRedirectUri,
				requiresSecretProxy: provider.requires_secret_proxy,
				pkceRequired: provider.pkce_required,
				hasClientSecret: !!provider.client_secret,
			});

			// Web clients and providers with secrets must proxy token exchange through the API.
			if (shouldProxyProviderRequests(provider)) {
				const apiBaseUrl = await resolveApiBaseUrl(
					provider,
					overrideApiBaseUrl,
				);

				const proxyUrl = `${apiBaseUrl}/api/v1/oauth/token/${provider.id}`;
				const proxyBody = JSON.stringify({
					code,
					redirect_uri: callbackRedirectUri,
					code_verifier: provider.pkce_required ? codeVerifier : undefined,
				});

				console.log("[OAuth] Using API proxy for token exchange:", proxyUrl);

				const response = await runtime.httpPost(proxyUrl, proxyBody, {
					"Content-Type": "application/json",
					Accept: "application/json",
				});

				if (!response.ok) {
					await response.text();
					console.error(
						"[OAuth] Proxy token exchange failed:",
						response.status,
					);
					throw new Error(`Token exchange failed: ${response.status}`);
				}

				const tokenData = (await response.json()) as {
					access_token: string;
					refresh_token?: string;
					expires_in?: number;
					token_type?: string;
					workspace_id?: string;
					workspace_name?: string;
					workspace_icon?: string;
					bot_id?: string;
					error?: string;
					error_description?: string;
				};

				if (tokenData.error) {
					throw new Error(
						`Token exchange error: ${tokenData.error} - ${tokenData.error_description || ""}`,
					);
				}

				if (!tokenData.access_token) {
					throw new Error("No access_token in token response");
				}

				return tokenData;
			}

			// Standard token exchange (non-proxy flow)
			const params = new URLSearchParams({
				code,
				redirect_uri: callbackRedirectUri,
				grant_type: "authorization_code",
			});

			// For providers with client_secret (like Notion), use Basic Auth
			// For PKCE-based providers (like Google), include client_id in body
			const headers: Record<string, string> = {
				"Content-Type": "application/x-www-form-urlencoded",
				// GitHub and some other providers return form-urlencoded by default
				// Request JSON explicitly for easier parsing
				Accept: "application/json",
			};

			if (provider.client_secret) {
				// Use HTTP Basic Authentication (Notion-style)
				const credentials = btoa(
					`${provider.client_id}:${provider.client_secret}`,
				);
				headers["Authorization"] = `Basic ${credentials}`;
			} else {
				// Include client_id in body for PKCE flows
				params.set("client_id", provider.client_id);
			}

			if (provider.pkce_required) {
				params.set("code_verifier", codeVerifier);
			}

			console.log("[OAuth] Token exchange request:", {
				url: provider.token_url,
				providerId: provider.id,
				hasClientSecret: !!provider.client_secret,
				pkceRequired: provider.pkce_required,
				clientId: provider.client_id,
				redirectUri: callbackRedirectUri,
			});

			const response = await runtime.httpPost(
				provider.token_url,
				params.toString(),
				headers,
			);

			console.log(
				"[OAuth] Token exchange response status:",
				response.status,
				response.ok,
			);

			if (!response.ok) {
				await response.text();
				console.error("[OAuth] Token exchange failed:", response.status);
				throw new Error(`Token exchange failed: ${response.status}`);
			}

			const responseText = await response.text();

			// Try to parse as JSON, fall back to form-urlencoded parsing
			let tokenData: {
				access_token: string;
				refresh_token?: string;
				expires_in?: number;
				token_type?: string;
				id_token?: string;
				workspace_id?: string;
				workspace_name?: string;
				workspace_icon?: string;
				bot_id?: string;
				owner?: unknown;
				error?: string;
				error_description?: string;
			};

			try {
				tokenData = JSON.parse(responseText);
			} catch {
				// GitHub and some providers may return form-urlencoded despite Accept header
				console.log("[OAuth] Response is not JSON, parsing as form-urlencoded");
				const formData = new URLSearchParams(responseText);
				tokenData = {
					access_token: formData.get("access_token") ?? "",
					refresh_token: formData.get("refresh_token") ?? undefined,
					expires_in: formData.get("expires_in")
						? Number(formData.get("expires_in"))
						: undefined,
					token_type: formData.get("token_type") ?? undefined,
					error: formData.get("error") ?? undefined,
					error_description: formData.get("error_description") ?? undefined,
				};
			}

			console.log("[OAuth] Parsed token data:", {
				hasAccessToken: !!tokenData.access_token,
				hasRefreshToken: !!tokenData.refresh_token,
				tokenType: tokenData.token_type,
				error: tokenData.error,
			});

			if (tokenData.error) {
				throw new Error(
					`Token exchange error: ${tokenData.error} - ${tokenData.error_description || ""}`,
				);
			}

			if (!tokenData.access_token) {
				throw new Error("No access_token in token response");
			}

			return tokenData;
		},

		async refreshToken(
			provider: IOAuthProvider,
			token: IStoredOAuthToken,
		): Promise<IStoredOAuthToken> {
			if (!token.refresh_token) {
				throw new Error("No refresh token available");
			}

			// Web clients and providers with secrets must proxy refresh requests through the API.
			if (shouldProxyProviderRequests(provider)) {
				const apiBaseUrl = await resolveApiBaseUrl(provider);

				const proxyUrl = `${apiBaseUrl}/api/v1/oauth/refresh/${provider.id}`;
				const proxyBody = JSON.stringify({
					refresh_token: token.refresh_token,
				});

				console.log("[OAuth] Using API proxy for token refresh:", proxyUrl);

				const response = await runtime.httpPost(proxyUrl, proxyBody, {
					"Content-Type": "application/json",
					Accept: "application/json",
				});

				if (!response.ok) {
					await response.text();
					await tokenStore.deleteToken(provider.id);
					throw new Error(`Token refresh failed: ${response.status}`);
				}

				const tokenResponse = (await response.json()) as {
					access_token: string;
					refresh_token?: string;
					expires_in?: number;
					token_type?: string;
				};

				const updatedToken: IStoredOAuthToken = {
					...token,
					access_token: tokenResponse.access_token,
					refresh_token: tokenResponse.refresh_token ?? token.refresh_token,
					expires_at: tokenResponse.expires_in
						? Date.now() + tokenResponse.expires_in * 1000
						: undefined,
					token_type: tokenResponse.token_type ?? token.token_type,
					storedAt: Date.now(),
				};

				await tokenStore.setToken(updatedToken);
				return updatedToken;
			}

			// Standard refresh flow (non-proxy)
			const params = new URLSearchParams({
				refresh_token: token.refresh_token,
				grant_type: "refresh_token",
			});

			// For providers with client_secret (like Notion), use Basic Auth
			const headers: Record<string, string> = {
				"Content-Type": "application/x-www-form-urlencoded",
				Accept: "application/json",
			};

			if (provider.client_secret) {
				const credentials = btoa(
					`${provider.client_id}:${provider.client_secret}`,
				);
				headers["Authorization"] = `Basic ${credentials}`;
			} else {
				params.set("client_id", provider.client_id);
			}

			const response = await runtime.httpPost(
				provider.token_url,
				params.toString(),
				headers,
			);

			if (!response.ok) {
				await response.text();
				await tokenStore.deleteToken(provider.id);
				throw new Error(`Token refresh failed: ${response.status}`);
			}

			const tokenResponse = (await response.json()) as {
				access_token: string;
				refresh_token?: string;
				expires_in?: number;
				token_type?: string;
			};

			const updatedToken: IStoredOAuthToken = {
				...token,
				access_token: tokenResponse.access_token,
				refresh_token: tokenResponse.refresh_token ?? token.refresh_token,
				expires_at: tokenResponse.expires_in
					? Date.now() + tokenResponse.expires_in * 1000
					: undefined,
				token_type: tokenResponse.token_type ?? token.token_type,
				storedAt: Date.now(),
			};

			await tokenStore.setToken(updatedToken);
			return updatedToken;
		},

		async fetchUserInfo(
			provider: IOAuthProvider,
			accessToken: string,
			overrideApiBaseUrl?: string,
		): Promise<IStoredOAuthToken["userInfo"]> {
			if (!provider.userinfo_url) {
				throw new Error(`Provider ${provider.id} does not expose userinfo_url`);
			}

			const response = shouldProxyProviderRequests(provider)
				? await runtime.httpPost(
						`${await resolveApiBaseUrl(provider, overrideApiBaseUrl)}/api/v1/oauth/userinfo/${provider.id}`,
						JSON.stringify({ access_token: accessToken }),
						{
							"Content-Type": "application/json",
							Accept: "application/json",
						},
					)
				: await runtime.httpGet(provider.userinfo_url, {
						Authorization: `Bearer ${accessToken}`,
					});

			if (!response.ok) {
				throw new Error(`Failed to fetch user info: ${response.status}`);
			}

			const data = (await response.json()) as {
				sub?: string;
				email?: string;
				name?: string;
				picture?: string;
			};
			return {
				sub: data.sub,
				email: data.email,
				name: data.name,
				picture: data.picture,
			};
		},

		async revokeToken(
			provider: IOAuthProvider,
			token: IStoredOAuthToken,
		): Promise<void> {
			if (!provider.revoke_url) {
				await tokenStore.deleteToken(provider.id);
				return;
			}

			try {
				if (shouldProxyProviderRequests(provider)) {
					const proxyUrl = `${await resolveApiBaseUrl(provider)}/api/v1/oauth/revoke/${provider.id}`;
					await runtime.httpPost(
						proxyUrl,
						JSON.stringify({ token: token.access_token }),
						{
							"Content-Type": "application/json",
							Accept: "application/json",
						},
					);
				} else {
					const params = new URLSearchParams({
						token: token.access_token,
						client_id: provider.client_id,
					});

					await runtime.httpPost(provider.revoke_url, params.toString(), {
						"Content-Type": "application/x-www-form-urlencoded",
					});
				}
			} catch {
				console.warn("Token revocation failed");
			}

			await tokenStore.deleteToken(provider.id);
		},

		async getValidTokensForProviders(providers: IOAuthProvider[]): Promise<{
			valid: Map<string, IStoredOAuthToken>;
			missing: IOAuthProvider[];
			needsRefresh: Map<
				string,
				{ token: IStoredOAuthToken; provider: IOAuthProvider }
			>;
		}> {
			const valid = new Map<string, IStoredOAuthToken>();
			const missing: IOAuthProvider[] = [];
			const needsRefresh = new Map<
				string,
				{ token: IStoredOAuthToken; provider: IOAuthProvider }
			>();

			for (const provider of providers) {
				const token = await tokenStore.getToken(provider.id);

				if (!token) {
					missing.push(provider);
				} else if (tokenStore.isExpired(token)) {
					if (token.refresh_token) {
						needsRefresh.set(provider.id, { token, provider });
					} else {
						missing.push(provider);
					}
				} else {
					valid.set(provider.id, token);
				}
			}

			return { valid, missing, needsRefresh };
		},

		async ensureTokensForProviders(
			providers: IOAuthProvider[],
			onConsentRequired?: (providers: IOAuthProvider[]) => Promise<boolean>,
		): Promise<Map<string, IStoredOAuthToken>> {
			const { valid, missing, needsRefresh } =
				await this.getValidTokensForProviders(providers);

			for (const [providerId, { token, provider }] of needsRefresh) {
				try {
					const refreshed = await this.refreshToken(provider, token);
					valid.set(providerId, refreshed);
				} catch {
					console.warn(`Failed to refresh token for ${providerId}`);
					missing.push(provider);
				}
			}

			if (missing.length > 0) {
				if (onConsentRequired) {
					const consented = await onConsentRequired(missing);
					if (!consented) {
						throw new Error("User declined OAuth consent");
					}
				}

				throw new Error(
					`Missing OAuth tokens for: ${missing.map((p) => p.name).join(", ")}`,
				);
			}

			return valid;
		},

		// ========== Device Flow Methods (RFC 8628) ==========

		/**
		 * Start device authorization flow.
		 * Returns device code info that should be displayed to the user.
		 */
		async startDeviceAuthorization(
			provider: IOAuthProvider,
			options?: { additionalScopes?: string[] },
		): Promise<IDeviceAuthResponse> {
			if (!provider.device_auth_url) {
				throw new Error(
					`Provider ${provider.name} does not support device flow`,
				);
			}

			// Validate client_id is present
			if (!provider.client_id || provider.client_id.trim() === "") {
				throw new Error(
					`OAuth client_id is not configured for ${provider.name}. ` +
						`Please set the environment variable or provide it via input pin.`,
				);
			}

			// Use merged_scopes if available (includes node-required scopes), otherwise fall back to base scopes
			const scopes = [...(provider.merged_scopes ?? provider.scopes)];
			if (options?.additionalScopes) {
				for (const scope of options.additionalScopes) {
					if (!scopes.includes(scope)) {
						scopes.push(scope);
					}
				}
			}

			const params = new URLSearchParams({
				client_id: provider.client_id,
				scope: scopes.join(" "),
			});

			const response = shouldProxyProviderRequests(provider)
				? await runtime.httpPost(
						`${await resolveApiBaseUrl(provider)}/api/v1/oauth/device/start/${provider.id}`,
						JSON.stringify({ scope: scopes.join(" ") }),
						{
							"Content-Type": "application/json",
							Accept: "application/json",
						},
					)
				: await runtime.httpPost(provider.device_auth_url, params.toString(), {
						"Content-Type": "application/x-www-form-urlencoded",
						Accept: "application/json",
					});

			if (!response.ok) {
				const errorText = await response.text();
				throw new Error(
					`Device authorization failed: ${response.status} - ${errorText}`,
				);
			}

			return response.json() as Promise<IDeviceAuthResponse>;
		},

		/**
		 * Poll for device authorization completion.
		 * Call this in a loop with the interval from the device auth response.
		 * Returns null if still pending, throws on error, returns token on success.
		 */
		async pollDeviceAuthorization(
			provider: IOAuthProvider,
			deviceCode: string,
			scopes: string[],
		): Promise<IStoredOAuthToken | null> {
			const params = new URLSearchParams({
				client_id: provider.client_id,
				device_code: deviceCode,
				grant_type: "urn:ietf:params:oauth:grant-type:device_code",
			});

			const response = shouldProxyProviderRequests(provider)
				? await runtime.httpPost(
						`${await resolveApiBaseUrl(provider)}/api/v1/oauth/device/poll/${provider.id}`,
						JSON.stringify({
							device_code: deviceCode,
						}),
						{
							"Content-Type": "application/json",
							Accept: "application/json",
						},
					)
				: await runtime.httpPost(provider.token_url, params.toString(), {
						"Content-Type": "application/x-www-form-urlencoded",
						Accept: "application/json",
					});

			const data = (await response.json()) as {
				access_token?: string;
				refresh_token?: string;
				expires_in?: number;
				token_type?: string;
				error?: string;
				error_description?: string;
			};

			// Handle pending states
			if (data.error === "authorization_pending") {
				return null; // Still waiting for user
			}

			if (data.error === "slow_down") {
				return null; // Need to slow down polling
			}

			if (data.error) {
				throw new Error(
					`Device authorization failed: ${data.error}${data.error_description ? ` - ${data.error_description}` : ""}`,
				);
			}

			if (!data.access_token) {
				throw new Error("No access token in device authorization response");
			}

			// Success! Store the token
			let userInfo: IStoredOAuthToken["userInfo"];
			if (provider.userinfo_url && data.access_token) {
				try {
					userInfo = await this.fetchUserInfo(provider, data.access_token);
				} catch {
					console.warn("Failed to fetch OAuth user info");
				}
			}

			const token: IStoredOAuthToken = {
				providerId: provider.id,
				access_token: data.access_token,
				refresh_token: data.refresh_token,
				expires_at: data.expires_in
					? Date.now() + data.expires_in * 1000
					: undefined,
				token_type: data.token_type ?? "Bearer",
				scopes,
				storedAt: Date.now(),
				userInfo,
			};

			await tokenStore.setToken(token);
			return token;
		},

		/**
		 * Complete device flow with polling loop.
		 * Opens the verification URL and polls until complete or timeout.
		 */
		async completeDeviceFlow(
			provider: IOAuthProvider,
			options?: {
				additionalScopes?: string[];
				onUserCode?: (
					userCode: string,
					verificationUri: string,
					verificationUriComplete?: string,
				) => void;
				timeoutMs?: number;
			},
		): Promise<IStoredOAuthToken> {
			const deviceAuth = await this.startDeviceAuthorization(provider, {
				additionalScopes: options?.additionalScopes,
			});

			// Notify caller of the user code to display
			if (options?.onUserCode) {
				options.onUserCode(
					deviceAuth.user_code,
					deviceAuth.verification_uri,
					deviceAuth.verification_uri_complete,
				);
			}

			// Open the verification URL
			const verificationUrl =
				deviceAuth.verification_uri_complete ?? deviceAuth.verification_uri;
			await runtime.openUrl(verificationUrl);

			// Use merged_scopes if available (includes node-required scopes), otherwise fall back to base scopes
			const scopes = [...(provider.merged_scopes ?? provider.scopes)];
			if (options?.additionalScopes) {
				for (const scope of options.additionalScopes) {
					if (!scopes.includes(scope)) {
						scopes.push(scope);
					}
				}
			}

			const timeoutMs = options?.timeoutMs ?? deviceAuth.expires_in * 1000;
			const startTime = Date.now();
			let interval = deviceAuth.interval * 1000;

			while (Date.now() - startTime < timeoutMs) {
				await new Promise((resolve) => setTimeout(resolve, interval));

				try {
					const token = await this.pollDeviceAuthorization(
						provider,
						deviceAuth.device_code,
						scopes,
					);
					if (token) {
						return token;
					}
				} catch (e) {
					// Check if we need to slow down
					if (e instanceof Error && e.message.includes("slow_down")) {
						interval += 5000; // Add 5 seconds
						continue;
					}
					throw e;
				}
			}

			throw new Error("Device authorization timed out");
		},
	};
}

export type OAuthService = ReturnType<typeof createOAuthService>;
