"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { asArray } from "../../../../lib/response-shape";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import type { IRouteMapping } from "../../../../state/backend-state/route-state";
import { normalizeRoute } from "../model/fields";

function prettifyPath(route: string): string {
	const words = route
		.replace(/^\//, "")
		.replace(/-/g, " ")
		.replace(/\//g, " / ");
	return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The routed event's name, "Home" for "/", else the path with dashes and slashes read as words. */
export function routeLabelOf(
	route: string,
	eventName: string | undefined,
	homeLabel: string,
): string {
	if (eventName) return eventName;
	return route === "/" ? homeLabel : prettifyPath(route);
}

export interface RouteLabelInput {
	readonly routes: readonly string[];
	readonly mappings: readonly IRouteMapping[];
	readonly events: readonly Pick<IEvent, "id" | "name">[];
	readonly homeLabel: string;
}

/** Route → label for the given (normalised) routes. */
export function routeLabelsOf(input: RouteLabelInput) {
	const nameByRoute = new Map<string, string>();
	for (const mapping of input.mappings) {
		const event = input.events.find(
			(candidate) => candidate.id === mapping.eventId,
		);
		if (event?.name) nameByRoute.set(normalizeRoute(mapping.path), event.name);
	}
	return Object.fromEntries(
		input.routes.map((route) => [
			route,
			routeLabelOf(route, nameByRoute.get(route), input.homeLabel),
		]),
	);
}

/**
 * Labels for the form's route buttons. The app's routes and events are only read when the form has
 * routes and the host's backend is ready; until then (and on hosts that know no other events) the
 * label comes from the path.
 */
export function useRouteLabels(appId: string, routes: readonly string[]) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const ready = useBackendReady();
	const enabled = ready && !!appId && routes.length > 0;
	const routesQuery = useInvoke<IRouteMapping[], [string]>(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		enabled,
		[appId],
	);
	const eventsQuery = useInvoke<IEvent[], [string]>(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		enabled,
		[appId],
	);
	const homeLabel = t("workbench.shell.home", "Home");
	return useMemo(
		() =>
			routeLabelsOf({
				routes,
				mappings: asArray(routesQuery.data),
				events: asArray(eventsQuery.data),
				homeLabel,
			}),
		[routes, routesQuery.data, eventsQuery.data, homeLabel],
	);
}
