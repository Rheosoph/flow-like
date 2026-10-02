"use client";

import { useRouter, useSearchParams } from "next/navigation";
import {
	type MouseEvent,
	type ReactNode,
	createContext,
	createElement,
	useCallback,
	useContext,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	CopyRef,
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import {
	ACCOUNT_DEVICES_PATH,
	APP_DEVICES_PATH,
	devicesHref,
} from "./devices-href";
import {
	ACCOUNT_SCOPE,
	type DevicesHost,
	type NavigationMode,
	type ParsedDevicesRoute,
	isAppRoute,
	navigationMode,
	parseDevicesRoute,
	routeScope,
} from "./devices-route";
import {
	type ResolveCode,
	type ResolveFleet,
	resolveTarget,
} from "./resolve-target";

/** What the routing hook needs from the page's router: Next in the apps, memory in tests. */
export interface RouteHost {
	/** Current query string without the leading `?`. */
	search: string;
	push(href: string): void;
	replace(href: string): void;
}

export interface NavigateOptions {
	/** Absent: push between objects, sections and flows; replace for tab, filter, view, `by` and `step` (plan §2.6). */
	replace?: boolean;
	/** Open the route in another scope: an app's page from the account area, or the account area from an app. */
	scope?: DevicesScope;
}

export interface RouteLinkProps {
	href: string;
	onClick(event: MouseEvent<HTMLAnchorElement>): void;
}

export interface DevicesRouteApi {
	/** The page that hosts the area. */
	host: DevicesHost;
	scope: DevicesScope;
	route: DevicesRoute;
	/** True while the app page without a valid `id` hands over to the account area. */
	redirecting: boolean;
	/** Link target for anchors (`devicesHref` in this scope, or in `scope`). */
	href(route: DevicesRoute, scope?: DevicesScope): string;
	navigate(route: DevicesRoute, options?: NavigateOptions): void;
	/** Opens a page outside the area (an app's Events page) through the host's router: no page load, so key sessions stay open. */
	leave(href: string): void;
	/** Drops one-shot params (`action`, `focus`, `import`, …) from the URL without a history entry. */
	clearParam(...names: string[]): void;
	/** Why a deep link could not be opened; set by `resolveWith`. */
	resolveBanner: CopyRef<ResolveCode> | null;
	dismissResolveBanner(): void;
	/** Checks the current route against the loaded fleet; a bad target shows the banner and opens the nearest valid view. */
	resolveWith(fleet: ResolveFleet): void;
}

const APP_HOME: DevicesRoute = { screen: "app-devices", by: "device" };

function hostPath(host: DevicesHost): string {
	return host === "app" ? APP_DEVICES_PATH : ACCOUNT_DEVICES_PATH;
}

function withQuery(path: string, params: URLSearchParams): string {
	const query = params.toString();
	return query ? `${path}?${query}` : path;
}

/** In an app, a target that only exists account-wide falls back to the app's own page. */
function fallbackIn(scope: DevicesScope, fallback: DevicesRoute): DevicesRoute {
	return scope.kind === "app" && !isAppRoute(fallback) ? APP_HOME : fallback;
}

function useRouteController(
	host: DevicesHost,
	router: RouteHost,
): DevicesRouteApi {
	const { search } = router;
	const parsed = useMemo(() => parseDevicesRoute(search, host), [search, host]);
	const [resolveBanner, setResolveBanner] =
		useState<CopyRef<ResolveCode> | null>(null);
	const latest = useRef({ parsed, router });
	useLayoutEffect(() => {
		latest.current = { parsed, router };
	}, [parsed, router]);

	const redirecting = host === "app" && parsed.scope.kind === "account";
	useEffect(() => {
		if (redirecting) router.replace(devicesHref(parsed.route, ACCOUNT_SCOPE));
	}, [redirecting, parsed, router]);

	const href = useCallback(
		(route: DevicesRoute, scope?: DevicesScope) =>
			devicesHref(route, scope ?? parsed.scope),
		[parsed.scope],
	);

	const navigate = useCallback(
		(route: DevicesRoute, options?: NavigateOptions) => {
			const current = latest.current;
			const scope = options?.scope ?? current.parsed.scope;
			const target = devicesHref(route, scope);
			if (target === devicesHref(current.parsed.route, current.parsed.scope))
				return;
			const to: ParsedDevicesRoute = { scope: routeScope(route, scope), route };
			const mode: NavigationMode =
				options?.replace === undefined
					? navigationMode(current.parsed, to)
					: options.replace
						? "replace"
						: "push";
			if (mode === "push") {
				setResolveBanner(null);
				current.router.push(target);
			} else current.router.replace(target);
		},
		[],
	);

	const leave = useCallback((href: string) => {
		latest.current.router.push(href);
	}, []);

	const clearParam = useCallback(
		(...names: string[]) => {
			const current = latest.current.router;
			const params = new URLSearchParams(current.search);
			const present = names.filter((name) => params.has(name));
			if (present.length === 0) return;
			for (const name of present) params.delete(name);
			current.replace(withQuery(hostPath(host), params));
		},
		[host],
	);

	const resolveWith = useCallback((fleet: ResolveFleet) => {
		const current = latest.current;
		const { scope, route } = current.parsed;
		const result = resolveTarget(route, fleet);
		if (result.ok) {
			if (result.changed)
				current.router.replace(devicesHref(result.route, scope));
			return;
		}
		setResolveBanner(result.banner);
		current.router.replace(
			devicesHref(fallbackIn(scope, result.fallback), scope),
		);
	}, []);

	const dismissResolveBanner = useCallback(() => setResolveBanner(null), []);

	return useMemo(
		() => ({
			host,
			scope: parsed.scope,
			route: parsed.route,
			redirecting,
			href,
			navigate,
			leave,
			clearParam,
			resolveBanner,
			dismissResolveBanner,
			resolveWith,
		}),
		[
			host,
			parsed,
			redirecting,
			href,
			navigate,
			leave,
			clearParam,
			resolveBanner,
			dismissResolveBanner,
			resolveWith,
		],
	);
}

const DevicesRouteContext = createContext<DevicesRouteApi | null>(null);

/** Binds the area to the Next router. Needs a `Suspense` boundary above it (static export + `useSearchParams`). */
export function DevicesRouteProvider({
	host,
	children,
}: Readonly<{ host: DevicesHost; children: ReactNode }>) {
	const params = useSearchParams();
	const router = useRouter();
	const search = params?.toString() ?? "";
	const routeHost = useMemo<RouteHost>(
		() => ({
			search,
			push: (href) => router.push(href, { scroll: false }),
			replace: (href) => router.replace(href, { scroll: false }),
		}),
		[search, router],
	);
	const api = useRouteController(host, routeHost);
	return createElement(DevicesRouteContext.Provider, { value: api }, children);
}

export interface MemoryNavigation {
	mode: NavigationMode;
	href: string;
}

/** The same routing over an in-memory URL: DOM tests and component galleries, no Next router needed. */
export function MemoryDevicesRoute({
	host = "account",
	initialSearch = "",
	onNavigate,
	children,
}: Readonly<{
	host?: DevicesHost;
	/** Query string, with or without the leading `?`. */
	initialSearch?: string;
	onNavigate?(navigation: MemoryNavigation): void;
	children: ReactNode;
}>) {
	const [search, setSearch] = useState(() => initialSearch.replace(/^\?/, ""));
	const routeHost = useMemo<RouteHost>(() => {
		const go = (mode: NavigationMode) => (href: string) => {
			const [path, query = ""] = href.split("?");
			onNavigate?.({ mode, href });
			if (path === hostPath(host)) setSearch(query);
		};
		return { search, push: go("push"), replace: go("replace") };
	}, [search, host, onNavigate]);
	const api = useRouteController(host, routeHost);
	return createElement(DevicesRouteContext.Provider, { value: api }, children);
}

/** `DevicesArea` mounts this: the Next router, unless a `MemoryDevicesRoute` above already provides the route. */
export function DevicesRouteBoundary({
	host,
	children,
}: Readonly<{ host: DevicesHost; children: ReactNode }>) {
	const outer = useContext(DevicesRouteContext);
	if (outer) return children;
	return createElement(DevicesRouteProvider, { host, children });
}

/** Route, scope and navigation of the Devices area. Mounted by `DevicesArea`; tests wrap in `MemoryDevicesRoute`. */
export function useDevicesRoute(): DevicesRouteApi {
	const api = useContext(DevicesRouteContext);
	if (!api)
		throw new Error(
			"useDevicesRoute() needs DevicesArea (or MemoryDevicesRoute in tests) above it.",
		);
	return api;
}

function isPlainClick(event: MouseEvent<HTMLAnchorElement>): boolean {
	return (
		!event.defaultPrevented &&
		event.button === 0 &&
		!(event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
	);
}

/** `<a {...link(route)}>`: a real link (new tab with a modifier key) that navigates inside the area on a plain click. */
export function useRouteLink(): (
	route: DevicesRoute,
	options?: NavigateOptions,
) => RouteLinkProps {
	const { href, navigate } = useDevicesRoute();
	return useCallback(
		(route, options) => ({
			href: href(route, options?.scope),
			onClick: (event) => {
				if (!isPlainClick(event)) return;
				event.preventDefault();
				navigate(route, options);
			},
		}),
		[href, navigate],
	);
}

/** `<a {...hostLink(appEventsHref(appId))}>`: a real link to a page outside the area that a plain click opens without a page load. */
export function useHostLink(): (href: string) => RouteLinkProps {
	const { leave } = useDevicesRoute();
	return useCallback(
		(href) => ({
			href,
			onClick: (event) => {
				if (!isPlainClick(event)) return;
				event.preventDefault();
				leave(href);
			},
		}),
		[leave],
	);
}
