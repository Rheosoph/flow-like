"use client";
import {
	type IEvent,
	type IOAuthProvider,
	type IStoredOAuthToken,
	useBackend,
	useHub,
	useInvoke,
} from "@tm9657/flow-like-ui";
import EventsPage from "@tm9657/flow-like-ui/components/settings/events/events-page";
import { useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";
import { EVENT_CONFIG } from "../../../../lib/event-config";
import { oauthConsentStore, oauthTokenStore } from "../../../../lib/oauth-db";
import { oauthService } from "../../../../lib/oauth-service";

export default function Page() {
	const backend = useBackend();
	const { hub } = useHub();
	const searchParams = useSearchParams();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

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
		Object.values(EVENT_CONFIG).forEach((cfg: any) => {
			Object.keys(cfg?.useInterfaces ?? {}).forEach((t) => set.add(t));
		});
		return Array.from(set);
	}, []);

	const newEventTemplate = useMemo<Partial<IEvent> | undefined>(() => {
		const raw = searchParams.get("newEvent");
		if (!raw) return undefined;
		try {
			return JSON.parse(decodeURIComponent(raw)) as Partial<IEvent>;
		} catch (err) {
			console.warn("invalid ?newEvent= payload", err);
			return undefined;
		}
	}, [searchParams]);

	return (
		<EventsPage
			eventMapping={EVENT_CONFIG}
			uiEventTypes={uiEventTypes}
			tokenStore={oauthTokenStore}
			consentStore={oauthConsentStore}
			onStartOAuth={handleStartOAuth}
			onRefreshToken={handleRefreshToken}
			hub={hub}
			newEventTemplate={newEventTemplate}
		/>
	);
}
