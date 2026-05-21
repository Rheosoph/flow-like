"use client";

import {
	OAuthExecutionProvider as BaseOAuthExecutionProvider,
	type IOAuthProvider,
	type IStoredOAuthToken,
	useBackend,
	useInvoke,
	useOAuthExecutionContext,
} from "@flow-like/flow-like-ui";
import { type ReactNode, useMemo, useRef } from "react";
import { oauthConsentStore, oauthTokenStore } from "../lib/oauth-db";
import { getOAuthService } from "../lib/oauth-service";
import { tauriOAuthRuntime } from "../lib/tauri-oauth-runtime";
import {
	clearProviderCache,
	setProviderCache,
	useOAuthCallbackListener,
} from "./oauth-callback-handler";

// Re-export the hook for convenience
export { useOAuthExecutionContext as useOAuthExecution };

export function OAuthExecutionProvider({ children }: { children: ReactNode }) {
	const providerCacheRef = useRef<Map<string, IOAuthProvider>>(new Map());
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	// Build API base URL from hub domain
	const apiBaseUrl = useMemo(() => {
		const hub = profile.data?.hub;
		if (!hub) return undefined;
		if (hub.startsWith("http://") || hub.startsWith("https://")) {
			return hub;
		}
		return `https://${hub}`;
	}, [profile.data?.hub]);

	// Create OAuth service with API base URL for secret proxy
	const oauthService = useMemo(() => getOAuthService(apiBaseUrl), [apiBaseUrl]);

	// Sync provider cache with the OAuth callback handler
	const handleProviderCacheUpdate = () => {
		if (providerCacheRef.current.size > 0) {
			setProviderCache(providerCacheRef.current);
		} else {
			clearProviderCache();
		}
	};

	return (
		<BaseOAuthExecutionProvider
			oauthService={oauthService}
			runtime={tauriOAuthRuntime}
			tokenStore={oauthTokenStore}
			consentStore={oauthConsentStore}
			providerCacheRef={providerCacheRef}
			onOAuthCallback={(providerId, token) => {
				handleProviderCacheUpdate();
			}}
		>
			<OAuthCallbackSync providerCacheRef={providerCacheRef}>
				{children}
			</OAuthCallbackSync>
		</BaseOAuthExecutionProvider>
	);
}

// Internal component to sync OAuth callbacks with the provider
function OAuthCallbackSync({
	children,
	providerCacheRef,
}: {
	children: ReactNode;
	providerCacheRef: React.MutableRefObject<Map<string, IOAuthProvider>>;
}) {
	const { handleOAuthCallback } = useOAuthExecutionContext();

	// Listen for OAuth callbacks and update the provider state
	useOAuthCallbackListener(
		(pending, token) => {
			// Update provider cache for handler
			if (providerCacheRef.current.size > 0) {
				setProviderCache(providerCacheRef.current);
			}
			// Notify the base provider to update authorizedProviders state
			handleOAuthCallback(pending.providerId, token as IStoredOAuthToken);
		},
		[providerCacheRef, handleOAuthCallback],
	);

	return <>{children}</>;
}
