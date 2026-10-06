"use client";

import { useCallback } from "react";
import { useClientRouter } from "../../../../lib/client-navigation";
import type { IUseInterfaceProps } from "../../interfaces";
import type { NavigateIntent } from "../contracts";
import { normalizeRoute } from "../model/fields";

export { normalizeRoute };

export type OnNavigate = NonNullable<IUseInterfaceProps["onNavigate"]>;

export interface NavigationRouter {
	push(href: string): void;
	replace(href: string): void;
}

/** `/use?id=<app>&route=<route>` for an app route; `/use…` and absolute URLs only gain the query parameters. */
export function buildUseNavigationUrl(
	appId: string,
	route: string,
	queryParams?: Readonly<Record<string, string>>,
): string {
	if (!route) {
		return `/use?id=${appId}&route=/`;
	}

	if (!route.startsWith("/use") && !route.startsWith("http")) {
		const [routePath, routeQueryString] = route.split("?");
		const params = new URLSearchParams();
		params.set("id", appId);
		params.set("route", normalizeRoute(routePath || "/"));
		params.delete("eventId");

		if (routeQueryString) {
			const routeParams = new URLSearchParams(routeQueryString);
			routeParams.forEach((value, key) => {
				params.set(key, value);
			});
		}

		if (queryParams) {
			for (const [key, value] of Object.entries(queryParams)) {
				params.set(key, value);
			}
		}

		return `/use?${params.toString()}`;
	}

	if (queryParams && Object.keys(queryParams).length > 0) {
		const params = new URLSearchParams(queryParams);
		const separator = route.includes("?") ? "&" : "?";
		return `${route}${separator}${params.toString()}`;
	}

	return route;
}

/** The host's `onNavigate` when it has one (hosted and standalone pages resolve routes themselves), else the client router. */
export function createNavigate(
	appId: string,
	onNavigate: OnNavigate | undefined,
	router: NavigationRouter,
) {
	return (intent: NavigateIntent) => {
		if (onNavigate) {
			onNavigate(intent.route, intent.replace, intent.queryParams);
			return;
		}
		const url = buildUseNavigationUrl(appId, intent.route, intent.queryParams);
		if (intent.replace) router.replace(url);
		else router.push(url);
	};
}

/** One navigate function for route buttons, the leave dialog and a2ui `navigateTo` intents from a run. */
export function useWorkbenchNavigate(
	appId: string,
	onNavigate: OnNavigate | undefined,
) {
	const router = useClientRouter();
	return useCallback(
		(intent: NavigateIntent) =>
			createNavigate(appId, onNavigate, router)(intent),
		[appId, onNavigate, router],
	);
}
