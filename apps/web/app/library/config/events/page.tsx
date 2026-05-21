"use client";
import {
	type IOAuthProvider,
	type IStoredOAuthToken,
	useBackend,
	useHub,
	useInvoke,
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
		Object.values(EVENT_CONFIG).forEach((cfg: any) => {
			Object.keys(cfg?.useInterfaces ?? {}).forEach((t) => set.add(t));
		});
		return Array.from(set);
	}, []);

	return (
		<EventsPage
			eventMapping={EVENT_CONFIG}
			uiEventTypes={uiEventTypes}
			tokenStore={oauthTokenStore}
			consentStore={oauthConsentStore}
			onStartOAuth={handleStartOAuth}
			onRefreshToken={handleRefreshToken}
			hub={hub}
		/>
	);
}
