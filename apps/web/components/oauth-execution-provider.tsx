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

export { useOAuthExecutionContext as useOAuthExecution };

export function OAuthExecutionProvider({ children }: { children: ReactNode }) {
	const providerCacheRef = useRef<Map<string, IOAuthProvider>>(new Map());
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const apiBaseUrl = useMemo(() => {
		const hub = profile.data?.hub;
		if (!hub) return undefined;
		if (hub.startsWith("http://") || hub.startsWith("https://")) {
			return hub;
		}
		return `https://${hub}`;
	}, [profile.data?.hub]);

	const oauthService = useMemo(() => getOAuthService(apiBaseUrl), [apiBaseUrl]);

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
			onOAuthCallback={() => {
				handleProviderCacheUpdate();
			}}
		>
			<OAuthCallbackSync providerCacheRef={providerCacheRef}>
				{children}
			</OAuthCallbackSync>
		</BaseOAuthExecutionProvider>
	);
}

function OAuthCallbackSync({
	children,
	providerCacheRef,
}: {
	children: ReactNode;
	providerCacheRef: React.MutableRefObject<Map<string, IOAuthProvider>>;
}) {
	const { handleOAuthCallback } = useOAuthExecutionContext();

	useOAuthCallbackListener(
		(pending, token) => {
			if (providerCacheRef.current.size > 0) {
				setProviderCache(providerCacheRef.current);
			}
			handleOAuthCallback(pending.providerId, token as IStoredOAuthToken);
		},
		[providerCacheRef, handleOAuthCallback],
	);

	return <>{children}</>;
}
