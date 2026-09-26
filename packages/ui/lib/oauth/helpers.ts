import { asArray } from "../response-shape";
import type {
	IBoard,
	IExecutionMode,
	IHub,
	INode,
	IOAuthProviderConfig,
} from "../schema";
import type {
	IOAuthProvider,
	IOAuthToken,
	IOAuthTokenCheckResult,
	IOAuthTokenStore,
	IStoredOAuthToken,
} from "./types";

/**
 * Check if a stored token has all the required scopes.
 * Returns true if the token has all required scopes, false otherwise.
 */
export function hasRequiredScopes(
	storedToken: IStoredOAuthToken,
	requiredScopes: string[],
): boolean {
	if (!requiredScopes || requiredScopes.length === 0) {
		return true;
	}

	const tokenScopes = new Set(storedToken.scopes ?? []);
	const missingScopes = requiredScopes.filter(
		(scope) => !tokenScopes.has(scope),
	);

	if (missingScopes.length > 0) {
		console.log(
			`[OAuth] Token for ${storedToken.providerId} is missing scopes:`,
			missingScopes,
			"| Has scopes:",
			storedToken.scopes,
			"| Required:",
			requiredScopes,
		);
		return false;
	}

	return true;
}
/**
 * Build a full IOAuthProvider from Hub config and node's provider_id + scopes.
 * Returns undefined if the provider is not configured in the Hub.
 */
export function buildOAuthProviderFromHub(
	providerId: string,
	nodeScopes: string[],
	hubConfig: IOAuthProviderConfig,
): IOAuthProvider {
	const baseScopes = asArray(hubConfig.scopes);
	const allScopes = [...new Set([...baseScopes, ...asArray(nodeScopes)])];

	return {
		id: providerId,
		name: hubConfig.name,
		auth_url: hubConfig.auth_url,
		token_url: hubConfig.token_url,
		client_id: hubConfig.client_id ?? "",
		scopes: baseScopes,
		pkce_required: hubConfig.pkce_required ?? false,
		requires_secret_proxy: hubConfig.requires_secret_proxy ?? false,
		revoke_url: hubConfig.revoke_url ?? undefined,
		userinfo_url: hubConfig.userinfo_url ?? undefined,
		audience: hubConfig.audience ?? undefined,
		device_auth_url: hubConfig.device_auth_url ?? undefined,
		use_device_flow: hubConfig.use_device_flow ?? false,
		use_implicit_flow: hubConfig.use_implicit_flow ?? false,
		merged_scopes: allScopes,
	};
}

/**
 * Extract raw OAuth requirements from a board (provider_id + scopes).
 * Useful for offline apps or when you only need the requirements without hub resolution.
 * Also returns requires_local_execution flag if any node has only_offline set,
 * execution_mode from the board settings, and can_execute_locally (always true since we have the board).
 */
export function extractOAuthRequirementsFromBoard(board: IBoard): {
	oauth_requirements: Array<{ provider_id: string; scopes: string[] }>;
	requires_local_execution: boolean;
	execution_mode: IExecutionMode;
	can_execute_locally: boolean;
} {
	const scopesMap = new Map<string, Set<string>>();
	let requiresLocalExecution = false;

	const processNode = (node: INode) => {
		if ((node as any).only_offline) {
			requiresLocalExecution = true;
		}

		const providerIds = (node as any).oauth_providers as string[] | undefined;
		if (providerIds && providerIds.length > 0) {
			for (const providerId of providerIds) {
				if (!scopesMap.has(providerId)) {
					scopesMap.set(providerId, new Set());
				}
			}
		}

		// Only add scopes for providers that are already registered via oauth_providers
		// required_oauth_scopes is informational - it documents what scopes a node needs
		// IF OAuth is used, but shouldn't trigger OAuth by itself
		const requiredScopes = (node as any).required_oauth_scopes as
			| Record<string, string[] | { values?: string[] }>
			| undefined;
		if (requiredScopes) {
			for (const [providerId, scopes] of Object.entries(requiredScopes)) {
				// Only add scopes if this provider was already registered by an OAuth provider node
				if (!scopesMap.has(providerId)) {
					continue;
				}
				const scopeSet = scopesMap.get(providerId)!;
				const scopeArray = Array.isArray(scopes)
					? scopes
					: (scopes?.values ?? []);
				for (const scope of scopeArray) {
					scopeSet.add(scope);
				}
			}
		}
	};

	for (const node of Object.values(board.nodes ?? {})) {
		processNode(node);
	}
	for (const layer of Object.values(board.layers ?? {})) {
		for (const node of Object.values(layer?.nodes ?? {})) {
			processNode(node);
		}
	}

	const oauth_requirements = Array.from(scopesMap.entries()).map(
		([provider_id, scopes]) => ({
			provider_id,
			scopes: Array.from(scopes),
		}),
	);

	return {
		oauth_requirements,
		requires_local_execution: requiresLocalExecution,
		execution_mode: board.execution_mode,
		can_execute_locally: true, // We have the board, so we can execute locally
	};
}

/**
 * Build OAuth providers from prerun requirements (provider_id + scopes).
 * Uses Hub config to get full provider details.
 * This is more efficient than extractOAuthProvidersFromBoard as it doesn't require the full board.
 */
export function buildOAuthProvidersFromPrerun(
	requirements: Array<{ provider_id: string; scopes: string[] }>,
	hub?: IHub,
): IOAuthProvider[] {
	const hubOAuthProviders = hub?.oauth_providers ?? {};
	const providers: IOAuthProvider[] = [];

	// Prerun answers come from the hub; a run must not die on a list an older hub omitted.
	for (const req of asArray(requirements)) {
		const hubConfig = hubOAuthProviders[req.provider_id];
		if (!hubConfig) {
			console.warn(
				`[OAuth] Provider ${req.provider_id} referenced but not configured in Hub`,
			);
			continue;
		}

		const provider = buildOAuthProviderFromHub(
			req.provider_id,
			req.scopes,
			hubConfig,
		);
		providers.push(provider);
	}

	return providers;
}

/**
 * Extract OAuth providers from board nodes (including layers).
 * Nodes only contain provider_id and scopes - full config comes from Hub.
 * Deduplicates providers by ID and merges required scopes from all nodes.
 */
export function extractOAuthProvidersFromBoard(
	board: IBoard,
	hub?: IHub,
): IOAuthProvider[] {
	const hubOAuthProviders = hub?.oauth_providers ?? {};
	const scopesMap = new Map<string, Set<string>>();

	const processNode = (node: INode) => {
		// oauth_providers is now just string[] of provider IDs
		const providerIds = (node as any).oauth_providers as string[] | undefined;
		if (providerIds && providerIds.length > 0) {
			for (const providerId of providerIds) {
				if (!scopesMap.has(providerId)) {
					scopesMap.set(providerId, new Set());
				}
			}
		}

		// Only add scopes for providers that are already registered via oauth_providers
		// required_oauth_scopes is informational - it documents what scopes a node needs
		// IF OAuth is used, but shouldn't trigger OAuth by itself
		const requiredScopes = (node as any).required_oauth_scopes as
			| Record<string, string[] | { values?: string[] }>
			| undefined;
		if (requiredScopes) {
			for (const [providerId, scopes] of Object.entries(requiredScopes)) {
				// Only add scopes if this provider was already registered by an OAuth provider node
				if (!scopesMap.has(providerId)) {
					continue;
				}
				const scopeSet = scopesMap.get(providerId)!;
				// Handle both array format and protobuf { values: [] } format
				const scopeArray = Array.isArray(scopes)
					? scopes
					: (scopes?.values ?? []);
				for (const scope of scopeArray) {
					scopeSet.add(scope);
				}
			}
		}
	};

	for (const node of Object.values(board.nodes ?? {})) {
		processNode(node);
	}
	for (const layer of Object.values(board.layers ?? {})) {
		for (const node of Object.values(layer?.nodes ?? {})) {
			processNode(node);
		}
	}

	// Build full providers from Hub config + collected scopes
	const providers: IOAuthProvider[] = [];
	for (const [providerId, nodeScopes] of scopesMap) {
		const hubConfig = hubOAuthProviders[providerId];
		if (!hubConfig) {
			console.warn(
				`[OAuth] Provider ${providerId} referenced by node but not configured in Hub`,
			);
			continue;
		}

		const provider = buildOAuthProviderFromHub(
			providerId,
			Array.from(nodeScopes),
			hubConfig,
		);
		providers.push(provider);
		console.log(
			`[OAuth] Built provider ${provider.name} with scopes:`,
			provider.merged_scopes,
		);
	}

	console.log(
		`[OAuth] Found ${providers.length} OAuth providers in board:`,
		providers,
	);
	return providers;
}

/**
 * Convert stored token to backend format.
 */
export function storedTokenToBackendFormat(
	token: IStoredOAuthToken,
): IOAuthToken {
	return {
		access_token: token.access_token,
		refresh_token: token.refresh_token,
		expires_at: token.expires_at
			? Math.floor(token.expires_at / 1000)
			: undefined,
		token_type: token.token_type ?? "Bearer",
	};
}

/**
 * Options for getOAuthTokensForProviders
 */
export interface GetOAuthTokensOptions {
	/**
	 * Optional function to refresh expired tokens.
	 * If provided, will attempt refresh before marking token as missing.
	 */
	refreshToken?: (
		provider: IOAuthProvider,
		token: IStoredOAuthToken,
	) => Promise<IStoredOAuthToken>;
}

/**
 * Get OAuth tokens for required providers, checking validity.
 * Returns valid tokens and list of providers that need authorization.
 * If refreshToken callback is provided, will attempt to refresh expired tokens.
 */
export async function getOAuthTokensForProviders(
	providers: IOAuthProvider[],
	tokenStore: IOAuthTokenStore,
	options?: GetOAuthTokensOptions,
): Promise<IOAuthTokenCheckResult> {
	const tokens: Record<string, IOAuthToken> = {};
	const missingProviders: IOAuthProvider[] = [];

	for (const provider of providers) {
		let storedToken = await tokenStore.getToken(provider.id);
		const requiredScopes = provider.merged_scopes ?? provider.scopes ?? [];

		// If token exists but is expired, try to refresh it
		if (storedToken && tokenStore.isExpired(storedToken)) {
			if (storedToken.refresh_token && options?.refreshToken) {
				try {
					console.log(
						`[OAuth] Token for ${provider.id} is expired, attempting refresh...`,
					);
					storedToken = await options.refreshToken(provider, storedToken);
					console.log(
						`[OAuth] Token for ${provider.id} refreshed successfully`,
					);
				} catch (e) {
					console.warn(
						`[OAuth] Failed to refresh token for ${provider.id}:`,
						e,
					);
					storedToken = undefined; // Mark as needing reauth
				}
			} else {
				console.log(
					`[OAuth] Token for ${provider.id} is expired and has no refresh token`,
				);
				storedToken = undefined;
			}
		}

		// Check if token exists, is valid, AND has all required scopes
		if (storedToken && !tokenStore.isExpired(storedToken)) {
			if (hasRequiredScopes(storedToken, requiredScopes)) {
				tokens[provider.id] = storedTokenToBackendFormat(storedToken);
			} else {
				console.log(
					`[OAuth] Token for ${provider.id} exists but is missing required scopes. Reauthorization needed.`,
				);
				missingProviders.push(provider);
			}
		} else {
			missingProviders.push(provider);
		}
	}

	return { tokens, missingProviders };
}

/**
 * Check OAuth tokens and return result with missing providers.
 * Does NOT throw - caller decides how to handle missing providers.
 * @param board The board to check for OAuth providers
 * @param tokenStore The token store to check for existing tokens
 * @param hub The hub configuration containing OAuth provider configs
 * @param options Optional configuration including refresh callback
 */
export async function checkOAuthTokens(
	board: IBoard,
	tokenStore: IOAuthTokenStore,
	hub?: IHub,
	options?: GetOAuthTokensOptions,
): Promise<IOAuthTokenCheckResult & { requiredProviders: IOAuthProvider[] }> {
	const requiredProviders = extractOAuthProvidersFromBoard(board, hub);

	if (requiredProviders.length === 0) {
		return { tokens: {}, missingProviders: [], requiredProviders: [] };
	}

	const result = await getOAuthTokensForProviders(
		requiredProviders,
		tokenStore,
		options,
	);
	return { ...result, requiredProviders };
}

/**
 * Check OAuth tokens using prerun requirements instead of full board.
 * This only requires execute permission, not read board permission.
 * @param requirements OAuth requirements from prerun endpoint
 * @param tokenStore The token store to check for existing tokens
 * @param hub The hub configuration containing OAuth provider configs
 * @param options Optional configuration including refresh callback
 */
export async function checkOAuthTokensFromPrerun(
	requirements: Array<{ provider_id: string; scopes: string[] }>,
	tokenStore: IOAuthTokenStore,
	hub?: IHub,
	options?: GetOAuthTokensOptions,
): Promise<IOAuthTokenCheckResult & { requiredProviders: IOAuthProvider[] }> {
	const requiredProviders = buildOAuthProvidersFromPrerun(requirements, hub);

	if (requiredProviders.length === 0) {
		return { tokens: {}, missingProviders: [], requiredProviders: [] };
	}

	const result = await getOAuthTokensForProviders(
		requiredProviders,
		tokenStore,
		options,
	);
	return { ...result, requiredProviders };
}

/**
 * Check if all required OAuth providers have valid tokens.
 * Throws an error if any are missing.
 * @param board The board to check for OAuth providers
 * @param tokenStore The token store to check for existing tokens
 * @param hub The hub configuration containing OAuth provider configs
 * @param options Optional configuration including refresh callback
 */
export async function ensureOAuthTokens(
	board: IBoard,
	tokenStore: IOAuthTokenStore,
	hub?: IHub,
	options?: GetOAuthTokensOptions,
): Promise<Record<string, IOAuthToken> | undefined> {
	const requiredProviders = extractOAuthProvidersFromBoard(board, hub);

	if (requiredProviders.length === 0) {
		return undefined;
	}

	const { tokens, missingProviders } = await getOAuthTokensForProviders(
		requiredProviders,
		tokenStore,
		options,
	);

	if (missingProviders.length > 0) {
		const missingNames = missingProviders.map((p) => p.name).join(", ");
		throw new Error(
			`Missing OAuth authorization for: ${missingNames}. Please authorize these services first.`,
		);
	}

	return tokens;
}
