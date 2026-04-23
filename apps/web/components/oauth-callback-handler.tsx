"use client";

import type {
	IOAuthCallbackData,
	IOAuthPendingAuth,
	IOAuthProvider,
	IStoredOAuthToken,
	OAuthService,
} from "@tm9657/flow-like-ui";
import { useCallback, useEffect } from "react";
import { toast } from "sonner";
import { oauthTokenStore } from "../lib/oauth-db";
import {
	broadcastOAuthCallbackCompletion,
	clearPendingOAuthCallback,
	OAUTH_CALLBACK_CHANNEL,
	OAUTH_CALLBACK_COMPLETE_KEY,
	parseOAuthCallbackCompletion,
	readPendingOAuthCallback,
} from "../lib/oauth-callback-storage";
import { getOAuthService } from "../lib/oauth-service";

type OAuthCallbackListener = (
	pending: IOAuthPendingAuth,
	token: Awaited<ReturnType<OAuthService["handleCallback"]>>,
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

function notifyListeners(
	pending: IOAuthPendingAuth,
	token: IStoredOAuthToken,
) {
	for (const listener of listeners) {
		try {
			listener(pending, token);
		} catch (e) {
			console.error("[Web OAuth] Callback listener error:", e);
		}
	}
}

async function processCallback(payload: IOAuthCallbackData): Promise<boolean> {
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

	console.log("[Web OAuth] Callback received:", {
		url,
		code,
		state,
		id_token: !!id_token,
		access_token: !!access_token,
		error,
	});

	if (error) {
		const errorMsg = error_description || error;
		console.error("[Web OAuth] Error:", errorMsg);
		toast.error(`Authorization failed: ${errorMsg}`);
		return false;
	}

	const isImplicitFlow = !!(access_token || id_token);
	const isCodeFlow = !!code;

	if (!isImplicitFlow && !isCodeFlow) {
		console.error("[Web OAuth] Invalid callback: no code or tokens received");
		toast.error("Invalid callback: no authorization data received");
		return false;
	}

	if (!state) {
		console.error("[Web OAuth] Missing state in callback");
		toast.error("Invalid callback: missing state parameter");
		return false;
	}

	try {
		const pending = await oauthTokenStore.getPendingAuth(state);

		if (!pending) {
			console.error("[Web OAuth] No pending auth found for state:", state);
			toast.error("Authorization session expired or invalid");
			return false;
		}

		const provider = pending.provider ?? providerCache?.get(pending.providerId);

		if (!provider) {
			console.error(
				"[Web OAuth] Provider not found in cache or pending auth:",
				pending.providerId,
			);
			toast.error(`Provider not found: ${pending.providerId}. Please retry.`);
			return false;
		}

		const oauthService = getOAuthService(pending.apiBaseUrl);
		let token: Awaited<ReturnType<OAuthService["handleCallback"]>>;

		if (isImplicitFlow) {
			token = await oauthService.handleImplicitCallback(pending, provider, {
				access_token: access_token!,
				id_token: id_token ?? undefined,
				token_type: token_type ?? "Bearer",
				expires_in: expires_in ? Number.parseInt(expires_in, 10) : undefined,
				scope: scope ?? undefined,
			});
		} else {
			token = await oauthService.handleCallback(url, provider);
		}

		console.log("[Web OAuth] Token obtained for provider:", provider.name);
		toast.success(`Connected to ${provider.name}`);

		notifyListeners(pending, token as IStoredOAuthToken);
		broadcastOAuthCallbackCompletion(
			pending,
			token as IStoredOAuthToken,
		);

		return true;
	} catch (e) {
		console.error("[Web OAuth] Failed to handle callback:", e);
		toast.error(
			`Authorization failed: ${e instanceof Error ? e.message : "Unknown error"}`,
		);
		return false;
	}
}

export function OAuthCallbackHandler({
	children,
}: {
	children: React.ReactNode;
}) {
	useEffect(() => {
		const seenCompletionTimestamps = new Set<number>();
		let channel: BroadcastChannel | null = null;

		const handleCompletion = (rawValue: string | null) => {
			if (!rawValue) {
				return;
			}

			const completion = parseOAuthCallbackCompletion(rawValue);
			if (!completion) {
				return;
			}

			if (seenCompletionTimestamps.has(completion.timestamp)) {
				return;
			}
			seenCompletionTimestamps.add(completion.timestamp);

			console.log(
				"[Web OAuth] Received cross-tab OAuth completion for provider:",
				completion.pending.providerId,
			);
			notifyListeners(completion.pending, completion.token);
		};

		const handleOAuthEvent = (event: Event) => {
			const customEvent = event as CustomEvent<IOAuthCallbackData>;
			const payload = customEvent.detail;
			if (!payload) return;

			console.log("[Web OAuth] Event received:", payload);
			void processCallback(payload).then((processed) => {
				if (processed) {
					clearPendingOAuthCallback();
				}
			});
		};

		const checkPendingCallback = () => {
			const data = readPendingOAuthCallback();
			if (!data) {
				return;
			}

			console.log("[Web OAuth] Processing pending callback from storage");
			void processCallback({
				url: data.url,
				code: data.code,
				state: data.state,
				id_token: data.id_token,
				access_token: data.access_token,
				token_type: data.token_type,
				expires_in: data.expires_in,
				scope: data.scope,
				error: null,
				error_description: null,
			}).then((processed) => {
				if (processed) {
					clearPendingOAuthCallback();
				}
			});
		};

		const handleStorageChange = (event: StorageEvent) => {
			if (event.key !== OAUTH_CALLBACK_COMPLETE_KEY) {
				return;
			}

			handleCompletion(event.newValue);
		};

		const handleChannelMessage = (event: MessageEvent) => {
			if (event.data?.type !== "oauth-complete") {
				return;
			}

			handleCompletion(JSON.stringify(event.data.payload));
		};

		window.addEventListener("thirdparty-oauth-callback", handleOAuthEvent);
		window.addEventListener("storage", handleStorageChange);

		try {
			channel = new BroadcastChannel(OAUTH_CALLBACK_CHANNEL);
			channel.addEventListener("message", handleChannelMessage);
		} catch {
			channel = null;
		}

		const timer = setTimeout(checkPendingCallback, 100);

		return () => {
			window.removeEventListener("thirdparty-oauth-callback", handleOAuthEvent);
			window.removeEventListener("storage", handleStorageChange);
			if (channel) {
				channel.removeEventListener("message", handleChannelMessage);
				channel.close();
			}
			clearTimeout(timer);
		};
	}, []);

	return <>{children}</>;
}
