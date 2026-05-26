"use client";

import { listen } from "@tauri-apps/api/event";
import type {
	IOAuthCallbackData,
	IOAuthPendingAuth,
	IOAuthProvider,
} from "@flow-like/flow-like-ui";
import { useCallback, useEffect } from "react";
import { toast } from "sonner";
import { oauthTokenStore } from "../lib/oauth-db";
import { oauthService } from "../lib/oauth-service";

type OAuthCallbackListener = (
	pending: IOAuthPendingAuth,
	token: Awaited<ReturnType<typeof oauthService.handleCallback>>,
) => void;

const listeners = new Set<OAuthCallbackListener>();

export function addOAuthCallbackListener(listener: OAuthCallbackListener) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

export function useOAuthCallbackListener(
	callback: OAuthCallbackListener,
	deps: React.DependencyList = [],
) {
	// biome-ignore lint/correctness/useExhaustiveDependencies: we want to allow custom deps
	const memoizedCallback = useCallback(callback, deps);

	useEffect(() => {
		const unsubscribe = addOAuthCallbackListener(memoizedCallback);
		return () => {
			unsubscribe();
		};
	}, [memoizedCallback]);
}

let providerCache: Map<string, IOAuthProvider> | null = null;

export function setProviderCache(providers: Map<string, IOAuthProvider>) {
	providerCache = providers;
}

export function clearProviderCache() {
	providerCache = null;
}

async function processCallback(payload: IOAuthCallbackData) {
	const {
		url,
		code,
		state,
		id_token,
		access_token,
		token_type,
		expires_in,
		scope,
		error,
		error_description,
	} = payload;

	console.log("OAuth/OIDC callback received:", {
		url,
		code,
		state,
		id_token: !!id_token,
		access_token: !!access_token,
		error,
	});

	if (error) {
		const errorMsg = error_description || error;
		console.error("OAuth/OIDC error:", errorMsg);
		toast.error(`Authorization failed: ${errorMsg}`);
		return;
	}

	// Determine flow type: implicit (has access_token directly) or authorization code (has code)
	const isImplicitFlow = !!(access_token || id_token);
	const isCodeFlow = !!code;

	if (!isImplicitFlow && !isCodeFlow) {
		console.error("Invalid callback: no code or tokens received");
		toast.error("Invalid callback: no authorization data received");
		return;
	}

	if (!state) {
		console.error("Missing state in callback");
		toast.error("Invalid callback: missing state parameter");
		return;
	}

	try {
		// Peek at the pending auth to get the provider ID
		const pending = await oauthTokenStore.getPendingAuth(state);

		if (!pending) {
			console.error("No pending auth found for state:", state);
			toast.error("Authorization session expired or invalid");
			return;
		}

		// Try to get provider from pending auth first (survives page reload),
		// then fall back to cache (for same-session callbacks)
		const provider = pending.provider ?? providerCache?.get(pending.providerId);

		if (!provider) {
			console.error(
				"Provider not found in cache or pending auth:",
				pending.providerId,
			);
			toast.error(`Provider not found: ${pending.providerId}. Please retry.`);
			return;
		}

		let token: Awaited<ReturnType<typeof oauthService.handleCallback>>;

		if (isImplicitFlow) {
			// OIDC Implicit flow: tokens are returned directly, no exchange needed
			token = await oauthService.handleImplicitCallback(pending, provider, {
				access_token: access_token!,
				id_token: id_token ?? undefined,
				token_type: token_type ?? "Bearer",
				expires_in: expires_in ? Number.parseInt(expires_in, 10) : undefined,
				scope: scope ?? undefined,
			});
		} else {
			// OAuth Authorization Code flow: exchange code for tokens
			token = await oauthService.handleCallback(url, provider);
		}

		console.log("Token obtained for provider:", provider.name);
		toast.success(`Connected to ${provider.name}`);

		for (const listener of listeners) {
			try {
				listener(pending, token);
			} catch (e) {
				console.error("Callback listener error:", e);
			}
		}
	} catch (e) {
		console.error("Failed to handle callback:", e);
		toast.error(
			`Authorization failed: ${e instanceof Error ? e.message : "Unknown error"}`,
		);
	}
}

export function OAuthCallbackHandler({
	children,
}: {
	children: React.ReactNode;
}) {
	useEffect(() => {
		// Debug event listener for development builds where deep links don't work
		// Usage: window.dispatchEvent(new CustomEvent("debug-thirdparty", { detail: { url: "..." } }))
		const handleDebugEvent = (event: Event) => {
			const customEvent = event as CustomEvent<{ url: string }>;
			const url = customEvent.detail?.url;
			if (!url) return;

			console.log("Debug thirdparty callback received:", url);

			// Parse the URL and extract params
			try {
				const parsedUrl = new URL(url);
				const params = new URLSearchParams(parsedUrl.search);

				// Also check hash for implicit flow
				if (parsedUrl.hash) {
					const hashParams = new URLSearchParams(parsedUrl.hash.substring(1));
					hashParams.forEach((v, k) => {
						if (!params.has(k)) params.set(k, v);
					});
				}

				const payload: IOAuthCallbackData = {
					url,
					code: params.get("code"),
					state: params.get("state"),
					id_token: params.get("id_token"),
					access_token: params.get("access_token"),
					token_type: params.get("token_type"),
					expires_in: params.get("expires_in"),
					scope: params.get("scope"),
					error: params.get("error"),
					error_description: params.get("error_description"),
				};

				processCallback(payload);
			} catch (e) {
				console.error("Failed to parse debug callback URL:", e);
			}
		};

		window.addEventListener("debug-thirdparty", handleDebugEvent);

		const unlisten = listen<IOAuthCallbackData>(
			"thirdparty/callback",
			async (event) => {
				await processCallback(event.payload);
			},
		);

		return () => {
			window.removeEventListener("debug-thirdparty", handleDebugEvent);
			unlisten.then((unsub) => unsub());
		};
	}, []);

	return <>{children}</>;
}
