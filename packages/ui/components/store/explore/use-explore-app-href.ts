"use client";

import { useRouter } from "next/navigation";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { asArray } from "../../../lib/response-shape";
import { useBackend } from "../../../state/backend-state";
import type { IEventMapping } from "../../interfaces/interfaces";
import { appPairs } from "../../library/library-types";

export function appStoreHref(appId: string): string {
	return `/store?id=${encodeURIComponent(appId)}`;
}

/** Owned apps open straight into `/use` when they have a usable route or event; everything else opens the store page. */
export function useExploreAppHref(eventConfig?: IEventMapping) {
	const backend = useBackend();
	const router = useRouter();
	const userApps = useInvoke(backend.appState.getApps, backend.appState, []);

	const userAppIds = useMemo(
		() => new Set(appPairs(userApps.data).map(([app]) => app.id)),
		[userApps.data],
	);

	const usableEvents = useMemo(() => {
		const set = new Set<string>();
		for (const config of Object.values(eventConfig ?? {})) {
			const usable = Object.keys(config.useInterfaces);
			for (const eventType of usable) {
				if (config.eventTypes.includes(eventType)) set.add(eventType);
			}
		}
		return set;
	}, [eventConfig]);

	const resolveUseHref = useCallback(
		async (appId: string) => {
			if (!userAppIds.has(appId)) return null;

			const [routes, events] = await Promise.all([
				backend.routeState.getRoutes(appId, true).catch(() => []),
				backend.eventState.getEvents(appId, true).catch(() => []),
			]);
			const activeEvents = asArray(events).filter((event) => event.active);
			const activeEventsById = new Map(
				activeEvents.map((event) => [event.id, event] as const),
			);

			const hasUsableRoute = asArray(routes).some((route) => {
				const routeEvent = activeEventsById.get(route.eventId);
				return Boolean(
					routeEvent?.default_page_id ||
						(routeEvent && usableEvents.has(routeEvent.event_type)),
				);
			});
			if (hasUsableRoute) return `/use?id=${appId}`;

			const fallbackEvent = activeEvents.find(
				(event) => event.default_page_id || usableEvents.has(event.event_type),
			);
			if (!fallbackEvent) return null;

			return `/use?id=${appId}&eventId=${fallbackEvent.id}`;
		},
		[backend.eventState, backend.routeState, usableEvents, userAppIds],
	);

	const openApp = useCallback(
		async (appId: string) => {
			const useHref = await resolveUseHref(appId);
			router.push(useHref ?? appStoreHref(appId));
		},
		[resolveUseHref, router],
	);

	const isOwned = useCallback(
		(appId: string) => userAppIds.has(appId),
		[userAppIds],
	);

	return { appHref: appStoreHref, openApp, isOwned, resolveUseHref };
}
