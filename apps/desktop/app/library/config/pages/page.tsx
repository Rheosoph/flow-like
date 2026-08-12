"use client";
import { useHub } from "@flow-like/flow-like-ui";
import type {
	IOAuthProvider,
	IStoredOAuthToken,
} from "@flow-like/flow-like-ui";
import EventsPage from "@flow-like/flow-like-ui/components/settings/events/events-page";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import { useCallback, useMemo } from "react";
import { oauthConsentStore, oauthTokenStore } from "../../../../lib/oauth-db";
import { oauthService } from "../../../../lib/oauth-service";

export default function Page() {
	const { hub } = useHub();

	const handleStartOAuth = useCallback(async (provider: IOAuthProvider) => {
		await oauthService.startAuthorization(provider);
	}, []);

	const handleRefreshToken = useCallback(
		async (provider: IOAuthProvider, token: IStoredOAuthToken) => {
			return oauthService.refreshToken(provider, token);
		},
		[],
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
