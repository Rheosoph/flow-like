import { type OAuthService, createOAuthService } from "@flow-like/flow-like-ui";
import { oauthTokenStore } from "./oauth-db";
import { tauriOAuthRuntime } from "./tauri-oauth-runtime";

export type { IOAuthProvider } from "@flow-like/flow-like-ui";

let cachedService: OAuthService | null = null;
let cachedApiBaseUrl: string | undefined;

export function getOAuthService(apiBaseUrl?: string): OAuthService {
	if (cachedService && cachedApiBaseUrl === apiBaseUrl) {
		return cachedService;
	}

	cachedApiBaseUrl = apiBaseUrl;
	cachedService = createOAuthService({
		runtime: tauriOAuthRuntime,
		tokenStore: oauthTokenStore,
		getApiBaseUrl: apiBaseUrl ? async () => apiBaseUrl : undefined,
	});

	return cachedService;
}

// Default service without API proxy (for backwards compatibility)
export const oauthService = createOAuthService({
	runtime: tauriOAuthRuntime,
	tokenStore: oauthTokenStore,
});
