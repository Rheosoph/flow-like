import { type OAuthService, createOAuthService } from "@flow-like/flow-like-ui";
import { oauthTokenStore } from "./oauth-db";
import { getPublicApiUrl } from "./public-config";
import { tauriOAuthRuntime } from "./tauri-oauth-runtime";
import { webOAuthPlatform } from "./web-oauth-runtime";

export type { IOAuthProvider } from "@flow-like/flow-like-ui";

let cachedService: OAuthService | null = null;
let cachedApiBaseUrl: string | undefined;

function normalizeApiBaseUrl(apiBaseUrl?: string | null): string | undefined {
	if (!apiBaseUrl) {
		return undefined;
	}
	const trimmed = apiBaseUrl.trim();
	if (!trimmed) {
		return undefined;
	}

	const normalized =
		trimmed.startsWith("http://") || trimmed.startsWith("https://")
			? trimmed
			: `https://${trimmed}`;

	return normalized.endsWith("/") ? normalized.slice(0, -1) : normalized;
}

export function getDefaultOAuthApiBaseUrl(): string {
	return normalizeApiBaseUrl(getPublicApiUrl()) ?? "https://api.flow-like.com";
}

export function getOAuthApiBaseUrl(hub?: string | null): string {
	return normalizeApiBaseUrl(hub) ?? getDefaultOAuthApiBaseUrl();
}

export function getOAuthService(apiBaseUrl?: string): OAuthService {
	// Always recalculate platform at runtime to ensure correct value
	const platform = webOAuthPlatform();
	const normalizedApiBaseUrl = normalizeApiBaseUrl(apiBaseUrl);

	if (cachedService && cachedApiBaseUrl === normalizedApiBaseUrl) {
		return cachedService;
	}

	cachedApiBaseUrl = normalizedApiBaseUrl;
	cachedService = createOAuthService({
		runtime: tauriOAuthRuntime,
		tokenStore: oauthTokenStore,
		getApiBaseUrl: async () =>
			normalizedApiBaseUrl ?? getDefaultOAuthApiBaseUrl(),
		platform,
	});

	return cachedService;
}

// Lazy getter for default service (avoids SSR issues with window)
let _oauthService: OAuthService | null = null;
export const oauthService: OAuthService = new Proxy({} as OAuthService, {
	get(_target, prop) {
		if (!_oauthService) {
			_oauthService = createOAuthService({
				runtime: tauriOAuthRuntime,
				tokenStore: oauthTokenStore,
				getApiBaseUrl: async () => getDefaultOAuthApiBaseUrl(),
				platform: webOAuthPlatform(),
			});
		}
		return Reflect.get(_oauthService, prop);
	},
});
