"use client";
import { useBackend, useHub, useInvoke } from "@flow-like/flow-like-ui";
import type {
	IOAuthProvider,
	IStoredOAuthToken,
} from "@flow-like/flow-like-ui";
import EventsPage from "@flow-like/flow-like-ui/components/settings/events/events-page";
import { useCallback, useMemo } from "react";
import { EVENT_CONFIG } from "../../../../lib/event-config";
import { oauthConsentStore, oauthTokenStore } from "../../../../lib/oauth-db";
import {
	getOAuthApiBaseUrl,
	getOAuthService,
} from "../../../../lib/oauth-service";

export default function Page() {
	const backend = useBackend();
	const { hub } = useHub();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const oauthService = useMemo(() => {
		return getOAuthService(getOAuthApiBaseUrl(profile.data?.hub));
	}, [profile.data?.hub]);

	const handleStartOAuth = useCallback(
		async (provider: IOAuthProvider) => {
			await oauthService.startAuthorization(provider);
		},
		[oauthService],
	);

	const handleRefreshToken = useCallback(
		async (provider: IOAuthProvider, token: IStoredOAuthToken) => {
			return oauthService.refreshToken(provider, token);
		},
		[oauthService],
	);

	const uiEventTypes = useMemo(() => {
		const set = new Set<string>();
		for (const config of Object.values(EVENT_CONFIG)) {
			for (const type of Object.keys(config?.useInterfaces ?? {})) {
				set.add(type);
			}
		}
		return Array.from(set);
	}, []);

	return (
		<main className="flex h-full max-h-full min-h-0 flex-col overflow-hidden">
			<div className="container mx-auto flex h-full min-h-0 flex-col px-6 py-4">
				<EventsPage
					eventMapping={EVENT_CONFIG}
					uiEventTypes={uiEventTypes}
					tokenStore={oauthTokenStore}
					consentStore={oauthConsentStore}
					onStartOAuth={handleStartOAuth}
					onRefreshToken={handleRefreshToken}
					hub={hub}
					basePath="/library/config/pages"
				/>
			</div>
		</main>
	);
}
