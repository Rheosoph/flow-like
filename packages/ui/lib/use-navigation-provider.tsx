"use client";

import { useRouter } from "next/navigation";
import { useCallback, useMemo } from "react";
import { ClientNavigationContext } from "./client-navigation";
import { QueryParamNavigationContext } from "./set-query-params";
import {
	isUsePathname,
	pathUseUrl,
	queryUseUrl,
	readUseRoutePath,
} from "./use-route-url";

// Custom Tauri schemes have opaque origins. Compare their complete authority
// instead of treating every URL with a "null" origin as local.
function isLocalUrl(url: URL): boolean {
	const current = new URL(window.location.href);
	return (
		url.protocol === current.protocol &&
		url.host === current.host &&
		url.username === current.username &&
		url.password === current.password
	);
}

export function UseNavigationProvider({
	children,
	routeMode = "path",
}: {
	children: React.ReactNode;
	routeMode?: "path" | "query";
}) {
	const router = useRouter();
	const href = useCallback(
		(value: string) => {
			if (typeof window === "undefined") return value;
			try {
				const url = new URL(value, window.location.href);
				if (!isLocalUrl(url) || !isUsePathname(url.pathname)) return value;
				return routeMode === "query" ? queryUseUrl(url) : pathUseUrl(url);
			} catch {
				return value;
			}
		},
		[routeMode],
	);
	const navigate = useCallback(
		(value: string, replace: boolean, options?: { scroll?: boolean }) => {
			const destination = href(value);
			if (routeMode === "query") {
				// Native assets export /use, so let Next navigate that shell instead
				// of restoring history entries for paths without exported payloads.
				router[replace ? "replace" : "push"](destination, options);
				return;
			}
			const url = new URL(destination, window.location.href);
			if (isLocalUrl(url) && isUsePathname(url.pathname)) {
				if (isUsePathname(window.location.pathname)) {
					const changedPath = url.pathname !== window.location.pathname;
					const changedHash = url.hash !== window.location.hash;
					// Keep the exported /use route mounted. Next's History API integration
					// updates its URL hooks without fetching an unexported route payload.
					window.history[replace ? "replaceState" : "pushState"](
						{ flowLikeUseScroll: options?.scroll !== false },
						"",
						destination,
					);
					if (changedPath && options?.scroll !== false) window.scrollTo(0, 0);
					if (!changedPath && changedHash && options?.scroll !== false) {
						try {
							const id = decodeURIComponent(url.hash.slice(1));
							const target =
								document.getElementById(id) ??
								document.getElementsByName(id)[0];
							if (target) target.scrollIntoView();
							else if (!id || id === "top") window.scrollTo(0, 0);
						} catch {
							// Malformed fragment escapes still remain valid browser URLs.
						}
					}
				} else {
					// Enter through the exported shell so desktop providers and native
					// listeners stay mounted. The page then canonicalizes its address.
					const shell = new URL(url);
					let route: string | undefined;
					try {
						route = readUseRoutePath(shell.pathname);
					} catch {
						// Let the page render its invalid-route screen for malformed links.
						window.location[replace ? "replace" : "assign"](destination);
						return;
					}
					if (route !== undefined) shell.searchParams.set("route", route);
					shell.pathname = "/use";
					router[replace ? "replace" : "push"](
						shell.pathname + shell.search + shell.hash,
						options,
					);
				}
				return;
			}
			router[replace ? "replace" : "push"](destination, options);
		},
		[href, router, routeMode],
	);
	const navigateQuery = useCallback(
		(value: string, replace: boolean) => {
			navigate(value + window.location.hash, replace, { scroll: false });
		},
		[navigate],
	);
	const navigation = useMemo(() => ({ href, navigate }), [href, navigate]);
	return (
		<ClientNavigationContext.Provider value={navigation}>
			<QueryParamNavigationContext.Provider value={navigateQuery}>
				{children}
			</QueryParamNavigationContext.Provider>
		</ClientNavigationContext.Provider>
	);
}
