import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import type { UserManagerSettings } from "oidc-client-ts";
import { getPublicApiUrl } from "./public-config";

export type WebOidcConfiguration = UserManagerSettings & {
	cognito?: { user_pool_id: string };
};

interface ConfigurationRequest {
	url: string;
	promise: Promise<WebOidcConfiguration | undefined>;
	reuseUntil: number;
}

// Only bridge the provider chunk load. Later auth mounts fetch current settings.
const PREFETCH_MAX_AGE_MS = 30_000;
let configurationRequest: ConfigurationRequest | undefined;

function clearRequest(request: ConfigurationRequest): void {
	if (configurationRequest === request) configurationRequest = undefined;
}

function requestConfiguration(prefetch: boolean) {
	const url = getApiUrl({ hub: getPublicApiUrl() }, "auth/openid");
	let request = configurationRequest;
	if (!request || request.url !== url || request.reuseUntil <= Date.now()) {
		request = {
			url,
			reuseUntil: Number.POSITIVE_INFINITY,
			promise: fetch(url, { method: "GET" }).then(async (response) => {
				if (!response.ok) {
					await response.text();
					console.error(`HTTP error: ${response.status}`);
					return undefined;
				}
				return (await response.json()) as WebOidcConfiguration;
			}),
		};
		configurationRequest = request;
		const started = request;
		void request.promise.then(
			(config) => {
				if (!config) clearRequest(started);
				else started.reuseUntil = Date.now() + PREFETCH_MAX_AGE_MS;
			},
			() => clearRequest(started),
		);
	}
	if (!prefetch) {
		const consumed = request;
		void request.promise.then(
			() => clearRequest(consumed),
			() => clearRequest(consumed),
		);
	}
	return request.promise;
}

/** Start the public request while account providers load, without loading OIDC. */
export function prefetchWebOidcConfiguration(): void {
	try {
		void requestConfiguration(true).catch(() => {});
	} catch {
		// WebAuthProvider owns the configuration error UI and can retry a failure.
	}
}

export async function loadWebOidcConfiguration() {
	return requestConfiguration(false);
}
