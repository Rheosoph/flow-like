"use client";

import { useTranslation } from "@flow-like/locales";
import { isEqual } from "lodash-es";
import { useRouter, useSearchParams } from "next/navigation";
import {
	type JSX,
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { useNetworkStatus } from "../../hooks/use-network-status";
import {
	boardReadinessKey,
	trackBoardReadiness,
} from "../../lib/board-readiness";
import {
	isPageContractDriftFor,
	subscribeToPageContractDrift,
} from "../../lib/page-contract-drift";
import { normalizeRoutePath, routePathsEqual } from "../../lib/route-path";
import { normalizeBoardVersion } from "../../lib/schema/flow/board-version";
import type { IEvent } from "../../lib/schema/flow/event";
import { useSetQueryParams } from "../../lib/set-query-params";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { useBackend } from "../../state/backend-state";
import type { IBoardState } from "../../state/backend-state/board-state";
import type {
	IGetPageOptions,
	IPage,
	IPageBootstrap,
	IPageState,
} from "../../state/backend-state/page-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import type { ISettingsProfile } from "../../types";
import { LoadingScreen } from "../ui/loading-screen";
import { Container } from "./container";
import {
	isSafeEmbeddedExternalHref,
	resolveEmbeddedPageNavigation,
} from "./embedded-page-navigation";
import { Header } from "./header";
import { InterfaceLoadError } from "./interface-load-error";
import type {
	ISidebarActions,
	IToolBarActions,
	IUseEventMapping,
	IUseInterfaceProps,
} from "./interfaces";
import { NoDefaultInterface } from "./no-default";
import { PageInterface } from "./page-interface";

/**
 * A page read can fail for reasons that resolve themselves: the payload still has to
 * reach this device, or the session is a moment away from being able to ask the server
 * for it. Retrying beats stranding a usable interface behind a dead end.
 */
const PAGE_RETRY_DELAYS_MS = [1_000, 3_000, 8_000];

/**
 * Floor between bootstrap revalidations. A Page whose contract is genuinely
 * broken raises a drift signal per rejected click; without a floor a user
 * clicking a dead button would refetch on every press.
 */
const BOOTSTRAP_REVALIDATE_MIN_MS = 3_000;

const unsupportedPageBootstrap = async (): Promise<IPageBootstrap> => {
	throw new Error("Page bootstrap is not supported by this backend");
};

function errorText(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === "string") return error;
	if (
		error &&
		typeof error === "object" &&
		"error" in error &&
		typeof (error as { error?: unknown }).error === "string"
	) {
		return (error as { error: string }).error;
	}
	return "Unknown error";
}

/**
 * The message the interface card shows.
 *
 * A page read that fails because its board is missing reports the missing file, while the
 * reason the board never arrived, such as a stale offline flag, a failed write, or a server that no
 * longer has it, travels as `cause`. Showing only the outer message turned every distinct
 * cause into the same undiagnosable "file not found", so the cause is appended when it adds
 * something the outer message does not already say.
 */
export function pageLoadErrorMessage(error: unknown): string {
	const message = errorText(error);
	const cause = error instanceof Error ? error.cause : undefined;
	if (cause === undefined || cause === null) return message;

	const causeMessage = errorText(cause);
	if (!causeMessage || message.includes(causeMessage)) return message;
	return `${message} (${causeMessage})`;
}

export interface UsePageContentProps {
	/**
	 * Only the runtime slice is read here. Taking the narrow type means `/use` can pass a
	 * mapping that never references a configuration panel, and so never loads one.
	 */
	eventConfig: IUseEventMapping;
	notFound?: ReactNode;
	appId?: string | null;
	routePath?: string | null;
	eventId?: string | null;
	queryParams?: Record<string, string>;
	/**
	 * Whether this interface is on screen. An embedded runtime that parks its host keeps the
	 * page mounted, so timed page work has to be told to idle rather than inferring it.
	 */
	active?: boolean;
	embedded?: boolean;
	/** Use an explicit event target before resolving the current route. Route navigation can
	 * restore normal route resolution by sending a null event id. */
	eventIdTakesPrecedence?: boolean;
	onNavigate?: (next: {
		routePath?: string | null;
		eventId?: string | null;
		queryParams?: Record<string, string>;
	}) => void;
	/** Report the Event and page that actually resolved after route navigation. */
	onResolvedPage?: (target: { eventId: string; pageId: string }) => void;
}

/**
 * Reads the page while its board refreshes alongside it.
 *
 * The refresh exists for the *run*, not the render: a device that has only ever synced the
 * board manifest needs the real board before it executes anything. Rendering needs only the
 * page payload, so waiting for a full board download before even asking for the page charged
 * every page open for a guarantee only execution consumes. The refresh is registered with
 * `trackBoardReadiness`, and `PageInterface` waits on it before running a workflow.
 *
 * Page files are still indexed through their board on native clients, so a read that fails
 * while the board is arriving gets the ordering it used to have: wait for the refresh, then
 * try once more. A failed refresh must not prevent page-state's own local/remote fallback from
 * running, and web pages do not require a local board at all.
 *
 * An event pinned to a board version reads that version's published page: the current page
 * file belongs to the draft board and may already have moved on.
 */
export async function loadPageWithBoardSync(
	boardState: Pick<IBoardState, "getBoard">,
	pageState: Pick<IPageState, "getPage">,
	appId: string,
	pageId: string,
	boardId?: string,
	boardVersion?: [number, number, number],
	options?: IGetPageOptions,
): Promise<IPage> {
	if (!boardId) {
		return pageState.getPage(appId, pageId, boardId, boardVersion, options);
	}

	const boardSync = trackBoardReadiness(
		boardReadinessKey(appId, boardId, boardVersion),
		() => boardState.getBoard(appId, boardId, boardVersion, true),
	);

	try {
		return await pageState.getPage(
			appId,
			pageId,
			boardId,
			boardVersion,
			options,
		);
	} catch (error) {
		await boardSync;
		try {
			return await pageState.getPage(
				appId,
				pageId,
				boardId,
				boardVersion,
				options,
			);
		} catch (retryError) {
			// The first failure is the one that describes why the page is unreadable; a retry
			// against a board that just arrived can only restate it less precisely. A retry
			// that failed *differently* knows something the first one did not, so it is kept
			// as the cause rather than discarded.
			if (
				error instanceof Error &&
				error.cause === undefined &&
				pageLoadErrorMessage(retryError) !== pageLoadErrorMessage(error)
			) {
				throw new Error(error.message, { cause: retryError });
			}
			throw error;
		}
	}
}

export interface IStoreRedirectState {
	readonly embedded: boolean;
	readonly authLoading: boolean;
	readonly hasAccessToken: boolean;
	readonly appInLocalProfile: boolean;
	readonly localProfileCheckPending: boolean;
	readonly remoteAppCheckPending: boolean;
	readonly remoteAppLoaded: boolean;
	readonly remoteAppFailed: boolean;
	readonly eventsLoaded: boolean;
	readonly eventsFailed: boolean;
	readonly eventsFetching: boolean;
	readonly offline: boolean;
}

/**
 * The store is a dead end for a running interface, so it is only reached when
 * this device positively cannot open the app: neither the local profiles nor
 * the hub know it, or its event catalog produced nothing to render.
 *
 * A refresh that failed while usable data survived must never eject a working
 * interface. Routes in particular are optional metadata whose absence simply
 * falls back to the default event.
 */
export function resolveStoreRedirect(state: IStoreRedirectState): {
	pending: boolean;
	redirect: boolean;
} {
	if (state.embedded) return { pending: false, redirect: false };

	if (
		state.authLoading ||
		state.localProfileCheckPending ||
		state.remoteAppCheckPending
	) {
		return { pending: true, redirect: false };
	}

	// Without a network every access check is inconclusive and the store itself is a
	// dead end, so ejecting there trades a possibly usable cached interface for a
	// guaranteed empty screen.
	if (state.offline) return { pending: false, redirect: false };

	const catalogUnavailable =
		state.eventsFailed && !state.eventsLoaded && !state.eventsFetching;

	if (state.appInLocalProfile) {
		return { pending: false, redirect: catalogUnavailable };
	}

	const hasNoAccess = state.hasAccessToken
		? state.remoteAppFailed && !state.remoteAppLoaded
		: true;

	return { pending: false, redirect: hasNoAccess || catalogUnavailable };
}

export interface IRouteResolution {
	readonly mapping: IRouteMapping | null;
	/**
	 * A route other than "/" was asked for and nothing matched it. The caller still
	 * receives the default mapping to render, but a miss is a misconfigured link or
	 * a route list this device has not synced. This is not a normal navigation.
	 */
	readonly missed: boolean;
}

/**
 * Pick the route mapping for a requested path.
 *
 * Paths are compared in canonical form so a route stored as `/config/` still answers
 * a link for `/config`; an unmatched path falls back to the app's default route,
 * which is what makes an unresolvable link look like an ordinary page load. The
 * fallback is reported through `missed` so callers can say so out loud.
 */
export function resolveRouteMapping(
	availableRoutes: readonly IRouteMapping[],
	routePath: string | null | undefined,
): IRouteResolution {
	const defaultRoute =
		availableRoutes.find((route) => routePathsEqual(route.path, "/")) ?? null;
	const requested = normalizeRoutePath(routePath);

	if (requested === "/") return { mapping: defaultRoute, missed: false };

	const matched =
		availableRoutes.find((route) => routePathsEqual(route.path, requested)) ??
		null;

	return matched
		? { mapping: matched, missed: false }
		: { mapping: defaultRoute, missed: true };
}

/**
 * Build the runtime route index from the event catalog.
 *
 * Routes are stored on the event row, so asking the route endpoint after the event catalog has
 * arrived only repeats the same read. Keep the first event for a canonical path when malformed or
 * legacy data contains duplicates. Explicit routes win over the synthesized "/" mapping for a
 * default event whose route field is missing, matching the route endpoint's persisted rows.
 */
export function deriveRouteMappings(
	events: readonly IEvent[] | null | undefined,
): IRouteMapping[] {
	const mappings = new Map<string, IRouteMapping>();
	const add = (path: string, eventId: string) => {
		const key = normalizeRoutePath(path);
		if (!mappings.has(key)) mappings.set(key, { path, eventId });
	};

	for (const event of events ?? []) {
		const path = event.route?.trim();
		if (path) add(path, event.id);
	}

	for (const event of events ?? []) {
		if (!event.is_default || event.route?.trim()) continue;
		add("/", event.id);
	}

	return [...mappings.values()];
}

/** An inactive Event is configuration data, never a runnable interface target. */
export function isUsableRuntimeEvent(
	event: IEvent | null | undefined,
	usableEventTypes: { has(value: string): boolean },
): boolean {
	return Boolean(
		event?.active &&
			(event.default_page_id || usableEventTypes.has(event.event_type)),
	);
}

/**
 * Match the bootstrap endpoint's precedence to the runtime URL resolver. A URL that only names an
 * Event must not accidentally resolve the app's root route first.
 */
export function runtimeBootstrapTarget(
	routePath: string,
	eventId: string | null,
	preferEventId: boolean,
): { route?: string; eventId?: string } {
	if (preferEventId && eventId) return { eventId };
	return { route: routePath };
}

export function UsePageContent({
	eventConfig,
	notFound,
	appId: appIdProp,
	routePath: routePathProp,
	eventId: eventIdProp,
	queryParams: queryParamsProp,
	active = true,
	embedded = false,
	eventIdTakesPrecedence = false,
	onNavigate,
	onResolvedPage,
}: Readonly<UsePageContentProps>) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const searchParams = useSearchParams();
	const router = useRouter();
	const auth = useAuth();
	const isOnline = useNetworkStatus();
	const hasAccessToken = Boolean(auth.user?.access_token);
	const backendNeedsSignIn = backend.capabilities().needsSignIn;
	const shouldWaitForPageBoardSync = backend.capabilities().canExecuteLocally;

	const appId = appIdProp ?? searchParams.get("id");
	const routePath = routePathProp ?? searchParams.get("route") ?? "/";
	const eventId = eventIdProp ?? searchParams.get("eventId");
	// A URL that names only an Event is a direct Event target. The implicit "/" fallback must
	// not replace it with an unrelated default route. When both are explicit, the route keeps
	// precedence unless an embedded caller opts into Event-first resolution.
	const preferEventId =
		eventIdTakesPrecedence ||
		Boolean(eventId && routePathProp == null && !searchParams.has("route"));
	const authCheckPending = Boolean(appId && !embedded && auth.isLoading);
	const bootstrapTarget = useMemo(
		() => runtimeBootstrapTarget(routePath, eventId, preferEventId),
		[routePath, eventId, preferEventId],
	);
	const supportsPageBootstrap = Boolean(backend.pageState.getPageBootstrap);
	const bootstrapEnabled = Boolean(
		appId &&
			(hasAccessToken || !backendNeedsSignIn) &&
			supportsPageBootstrap &&
			!auth.isLoading,
	);
	const bootstrap = useInvoke(
		backend.pageState.getPageBootstrap ?? unsupportedPageBootstrap,
		backend.pageState,
		[appId ?? "", bootstrapTarget.route, bootstrapTarget.eventId] as const,
		bootstrapEnabled,
		[auth.user?.profile?.sub ?? "anonymous"],
		0,
	);
	// Persisted query data is useful only after this mount has validated it against the endpoint.
	// `Cache-Control: no-cache` lets the browser turn an unchanged body into a cheap ETag round trip.
	//
	// A contract already validated on this mount survives a later REFETCH failure.
	// TanStack flips `isError` on a failed refetch while retaining `data`, and both
	// hosts run `networkMode: "always"`, so an offline revalidation genuinely runs
	// and fails — dropping to `undefined` there would take `pageExecutionRevision`
	// with it and swap a live, working Page for the "could not load its execution
	// authorization" card. Only a mount that never validated anything may be empty.
	//
	// Retained per target, so navigating to another route or Event never falls
	// back to the previous target's contract while its own fetch is in flight.
	const bootstrapTargetKey = `${appId ?? ""}|${bootstrapTarget.route ?? ""}|${bootstrapTarget.eventId ?? ""}`;
	const lastValidatedBootstrapRef = useRef<{
		key: string;
		data: IPageBootstrap;
	} | null>(null);
	if (bootstrap.isFetchedAfterMount && !bootstrap.isError && bootstrap.data) {
		lastValidatedBootstrapRef.current = {
			key: bootstrapTargetKey,
			data: bootstrap.data,
		};
	}
	const validatedBootstrap =
		bootstrap.isFetchedAfterMount && !bootstrap.isError
			? bootstrap.data
			: lastValidatedBootstrapRef.current?.key === bootstrapTargetKey
				? lastValidatedBootstrapRef.current.data
				: undefined;
	const bootstrapPending = Boolean(
		bootstrapEnabled && !bootstrap.isFetchedAfterMount && !bootstrap.isError,
	);

	const headerRef = useRef<IToolBarActions>(
		null,
	) as React.RefObject<IToolBarActions>;
	const sidebarRef = useRef<ISidebarActions>(
		null,
	) as React.RefObject<ISidebarActions>;
	const setQueryParams = useSetQueryParams();

	// --- Data fetching (force-fresh with offline fallback) ---

	const shouldLoadEventCatalog = Boolean(
		appId &&
			(!bootstrapEnabled ||
				bootstrap.isError ||
				(validatedBootstrap && !validatedBootstrap.page)),
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId ?? "", true],
		shouldLoadEventCatalog,
		[],
	);
	// The web query cache is persisted. While online, do not expose a restored catalog until this
	// mount has checked it; the bootstrap-selected Event remains available during that validation.
	const confirmedEventCatalog =
		supportsPageBootstrap && isOnline
			? events.isFetchedAfterMount && !events.isError
				? events.data
				: undefined
			: events.data;

	// Signed-in users open locally installed apps too, so the local profiles
	// stay authoritative for access even when a hub lookup is available.
	const needsFallbackAccessChecks = Boolean(
		!bootstrapEnabled || bootstrap.isError,
	);
	const needsLocalProfileCheck = Boolean(
		appId && !embedded && !auth.isLoading && needsFallbackAccessChecks,
	);

	const localProfiles = useInvoke(
		backend.userState.getAllSettingsProfiles,
		backend.userState,
		[],
		needsLocalProfileCheck,
	);

	const remoteApp = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId ?? ""],
		Boolean(appId && !embedded && hasAccessToken && needsFallbackAccessChecks),
	);

	const storeHref = useMemo(
		() => (appId ? `/store?id=${encodeURIComponent(appId)}` : "/store"),
		[appId],
	);

	const appIsInAnyLocalProfile = useMemo(() => {
		if (!appId) return false;
		const profiles = Array.isArray(localProfiles.data)
			? localProfiles.data
			: Object.values(
					(localProfiles.data ?? {}) as Record<string, ISettingsProfile>,
				);

		return profiles.some((profile) =>
			(profile.hub_profile?.apps ?? []).some((app) => app.app_id === appId),
		);
	}, [appId, localProfiles.data]);

	const localProfileCheckPending =
		bootstrapPending ||
		(needsLocalProfileCheck &&
			localProfiles.isFetching &&
			!localProfiles.data &&
			!localProfiles.isError);

	const needsAuthenticatedRemoteCheck = Boolean(
		appId && !embedded && hasAccessToken && needsFallbackAccessChecks,
	);
	const authenticatedRemoteCheckPending =
		bootstrapPending ||
		(needsAuthenticatedRemoteCheck &&
			remoteApp.isFetching &&
			!remoteApp.data &&
			!remoteApp.isError);

	const storeRedirect = useMemo(
		() =>
			resolveStoreRedirect({
				embedded: embedded || !appId,
				authLoading: authCheckPending,
				hasAccessToken,
				appInLocalProfile: appIsInAnyLocalProfile,
				localProfileCheckPending,
				remoteAppCheckPending: authenticatedRemoteCheckPending,
				remoteAppLoaded: Boolean(remoteApp.data || validatedBootstrap),
				remoteAppFailed: !validatedBootstrap && remoteApp.isError,
				eventsLoaded: Boolean(confirmedEventCatalog || validatedBootstrap),
				eventsFailed: !validatedBootstrap && events.isError,
				eventsFetching: bootstrapPending || events.isFetching,
				offline: !isOnline,
			}),
		[
			embedded,
			appId,
			authCheckPending,
			hasAccessToken,
			appIsInAnyLocalProfile,
			localProfileCheckPending,
			authenticatedRemoteCheckPending,
			remoteApp.data,
			remoteApp.isError,
			validatedBootstrap,
			bootstrapPending,
			confirmedEventCatalog,
			events.isError,
			events.isFetching,
			isOnline,
		],
	);
	const redirectCheckPending = storeRedirect.pending;
	const shouldRedirectToStore = storeRedirect.redirect;

	// One ejection per app: a redirect that re-fires while the replace is still
	// committing stacks navigations and flickers the interface back and forth.
	const redirectedAppRef = useRef<string | null>(null);
	const goToStore = useCallback(() => {
		if (!appId || embedded) return;
		// The store needs the network it is being used to escape to. Staying put keeps
		// whatever this device cached on screen until the connection returns.
		if (!isOnline) return;
		if (redirectedAppRef.current === appId) return;
		redirectedAppRef.current = appId;
		router.replace(storeHref);
	}, [appId, embedded, isOnline, router, storeHref]);

	// --- Computed: usable event types ---

	const usableEvents = useMemo(() => {
		const map = new Map<
			string,
			(props: IUseInterfaceProps) => JSX.Element | ReactNode | null
		>();
		for (const config of Object.values(eventConfig)) {
			for (const [eventType, useInterface] of Object.entries(
				config.useInterfaces,
			)) {
				if (config.eventTypes.includes(eventType)) {
					map.set(eventType, useInterface);
				}
			}
		}
		return map;
	}, [eventConfig]);

	const catalogEvents = useMemo(() => {
		const selected = validatedBootstrap?.event;
		// A persisted catalog may be shown after bootstrap itself failed, which preserves native and
		// offline fallback behavior. Once bootstrap succeeds, its freshly validated selected Event
		// wins over any older copy in that catalog.
		if (!selected) return confirmedEventCatalog;
		if (!confirmedEventCatalog) return [selected];
		return [
			selected,
			...confirmedEventCatalog.filter((event) => event.id !== selected.id),
		];
	}, [confirmedEventCatalog, validatedBootstrap]);

	const sortedEvents = useMemo(() => {
		if (!catalogEvents) return [];
		return catalogEvents
			.filter((a) => a.active)
			.toSorted((a, b) => a.priority - b.priority);
	}, [catalogEvents]);

	const availableRoutes = useMemo(
		() => deriveRouteMappings(catalogEvents),
		[catalogEvents],
	);

	const currentEvent = useMemo(() => {
		if (!eventId) return undefined;
		return sortedEvents.find((e) => e.id === eventId);
	}, [eventId, sortedEvents]);

	const canUseEvent = useCallback(
		(event: IEvent | null | undefined) =>
			isUsableRuntimeEvent(event, usableEvents),
		[usableEvents],
	);

	const routeKey = appId ? `${appId}:${routePath}` : "";
	const directEventKey = appId && eventId ? `${appId}:${eventId}` : "";

	// --- Route & event resolution ---

	const [routeMapping, setRouteMapping] = useState<IRouteMapping | null>(null);
	const [routeEvent, setRouteEvent] = useState<IEvent | null>(null);
	const [directEvent, setDirectEvent] = useState<IEvent | null>(null);
	const [pageData, setPageData] = useState<IPage | null>(null);
	const [pageError, setPageError] = useState<string | null>(null);
	const [pageRetry, setPageRetry] = useState<{ key: string; attempt: number }>({
		key: "",
		attempt: 0,
	});
	const [routeLoading, setRouteLoading] = useState(true);
	const [pageLoading, setPageLoading] = useState(false);
	const [resolvedRouteKey, setResolvedRouteKey] = useState("");
	const [resolvedDirectEventKey, setResolvedDirectEventKey] = useState("");
	const [resolvedPageKey, setResolvedPageKey] = useState("");

	const resolveKeyRef = useRef("");

	useEffect(() => {
		if (!appId) {
			setRouteMapping(null);
			setRouteEvent(null);
			setDirectEvent(null);
			setPageData(null);
			setPageError(null);
			setRouteLoading(false);
			setResolvedRouteKey("");
			setResolvedDirectEventKey("");
			setResolvedPageKey("");
			return;
		}

		const isNavigation = resolveKeyRef.current !== routeKey;
		const needsFreshResolution = resolvedRouteKey !== routeKey;
		resolveKeyRef.current = routeKey;

		// Only clear old state on actual navigation, not on data refreshes
		if (isNavigation) {
			setRouteMapping(null);
			setRouteEvent(null);
			setPageData(null);
			setResolvedPageKey("");
		}

		if (events.isFetching && !catalogEvents) {
			if (isNavigation || needsFreshResolution) {
				setRouteLoading(true);
			}
			return;
		}

		const { mapping, missed } = resolveRouteMapping(availableRoutes, routePath);

		if (missed) {
			console.warn(
				`[UsePage] No route matches "${routePath}" for app ${appId}; rendering ${
					mapping ? `the default route "${mapping.path}"` : "no route"
				} instead. Known routes: ${
					availableRoutes.map((route) => route.path).join(", ") || "(none)"
				}`,
			);
		}

		const cachedRouteEvent = mapping
			? (catalogEvents?.find((event) => event.id === mapping.eventId) ?? null)
			: null;

		if (!mapping || cachedRouteEvent) {
			setRouteMapping(mapping);
			setRouteEvent(cachedRouteEvent);
			setResolvedRouteKey(routeKey);
			setRouteLoading(false);
			return;
		}

		let cancelled = false;
		if (isNavigation || needsFreshResolution) {
			setRouteLoading(true);
		}

		const resolve = async () => {
			if (cancelled) return;

			try {
				let event: IEvent | null = null;
				if (mapping) {
					event = await backend.eventState.getEvent(appId, mapping.eventId);
				}
				if (cancelled) return;

				setRouteMapping(mapping);
				setRouteEvent(event);
			} catch (e) {
				if (cancelled) return;
				console.error("Failed to load route:", e);
				setRouteMapping(null);
				setRouteEvent(null);
			} finally {
				if (!cancelled) {
					setResolvedRouteKey(routeKey);
					setRouteLoading(false);
				}
			}
		};

		resolve();
		return () => {
			cancelled = true;
		};
	}, [
		appId,
		routePath,
		catalogEvents,
		events.isFetching,
		availableRoutes,
		backend.eventState,
		resolvedRouteKey,
		routeKey,
	]);

	const isRoutePending = Boolean(appId && resolvedRouteKey !== routeKey);

	useEffect(() => {
		if (redirectCheckPending || !shouldRedirectToStore) return;
		goToStore();
	}, [redirectCheckPending, shouldRedirectToStore, goToStore]);

	const effectiveRouteEvent = useMemo(() => {
		if (preferEventId && eventId) return null;
		return canUseEvent(routeEvent) ? routeEvent : null;
	}, [canUseEvent, eventId, preferEventId, routeEvent]);

	const effectiveRouteMapping = useMemo(() => {
		return effectiveRouteEvent ? routeMapping : null;
	}, [effectiveRouteEvent, routeMapping]);

	useEffect(() => {
		if (!appId || !eventId || effectiveRouteMapping) {
			setDirectEvent(null);
			setResolvedDirectEventKey("");
			return;
		}

		if (isRoutePending) {
			return;
		}

		if (currentEvent) {
			setDirectEvent(null);
			setResolvedDirectEventKey(directEventKey);
			return;
		}

		let cancelled = false;
		setDirectEvent(null);

		const resolve = async () => {
			try {
				const event = await backend.eventState.getEvent(appId, eventId);
				if (cancelled) return;

				setDirectEvent(event);
			} catch (e) {
				if (cancelled) return;
				console.error("Failed to load event:", e);
				setDirectEvent(null);
			} finally {
				if (!cancelled) {
					setResolvedDirectEventKey(directEventKey);
				}
			}
		};

		resolve();
		return () => {
			cancelled = true;
		};
	}, [
		appId,
		eventId,
		effectiveRouteMapping,
		isRoutePending,
		currentEvent,
		backend.eventState,
		directEventKey,
	]);

	const currentDirectEvent = useMemo(() => {
		if (!eventId || !directEventKey) return null;
		if (resolvedDirectEventKey !== directEventKey) return null;
		if (directEvent?.id !== eventId) return null;
		return directEvent;
	}, [directEvent, directEventKey, eventId, resolvedDirectEventKey]);

	const resolvedCurrentEvent = useMemo(
		() => currentEvent ?? currentDirectEvent ?? undefined,
		[currentEvent, currentDirectEvent],
	);

	const isDirectEventPending = Boolean(
		!isRoutePending &&
			appId &&
			eventId &&
			!effectiveRouteMapping &&
			!resolvedCurrentEvent &&
			resolvedDirectEventKey !== directEventKey,
	);

	// --- Active event ---

	const activeEvent = useMemo(() => {
		if (effectiveRouteEvent) return effectiveRouteEvent;
		return resolvedCurrentEvent;
	}, [effectiveRouteEvent, resolvedCurrentEvent]);

	const pageEvent = useMemo(() => {
		if (effectiveRouteEvent?.default_page_id) return effectiveRouteEvent;
		if (activeEvent?.default_page_id) return activeEvent;
		return null;
	}, [effectiveRouteEvent, activeEvent]);

	// Page targets suppress the application header, its only metadata consumer. Wait until the
	// Event catalog resolves before deciding, so a custom page never starts this unrelated read.
	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId ?? ""],
		Boolean(appId && catalogEvents && !pageEvent),
		[],
	);
	const pageEventId = pageEvent?.id ?? null;
	const pageId = pageEvent?.default_page_id ?? null;
	const pageBoardId = pageEvent?.board_id || undefined;
	const pageBoardVersion = useMemo(
		() => normalizeBoardVersion(pageEvent?.board_version),
		[pageEvent?.board_version],
	);

	const pageKey =
		appId && pageEventId && pageId
			? `${appId}:${pageEventId}:${pageId}:${pageBoardId ?? ""}:${pageBoardVersion?.join(".") ?? "latest"}`
			: "";
	const bootstrapPageData = useMemo(() => {
		if (!validatedBootstrap?.page || !pageEventId || !pageId) return null;
		if (validatedBootstrap.event.id !== pageEventId) return null;
		if (validatedBootstrap.page.id !== pageId) return null;
		return validatedBootstrap.page;
	}, [validatedBootstrap, pageEventId, pageId]);
	// A backend that supports governed Page bootstrap must never downgrade to
	// separately fetched Event/Page data when that bootstrap fails.
	const resolvedPageData = supportsPageBootstrap ? bootstrapPageData : pageData;
	const pageContentRevision = bootstrapPageData
		? (validatedBootstrap?.revision ?? undefined)
		: undefined;
	const pageExecutionRevision = bootstrapPageData
		? (validatedBootstrap?.executionRevision ?? undefined)
		: undefined;
	const pageExecutionAuthorityUnavailable = Boolean(
		pageEvent &&
			!bootstrapPending &&
			(!supportsPageBootstrap ||
				bootstrap.isError ||
				!bootstrapPageData ||
				!pageExecutionRevision),
	);

	useEffect(() => {
		if (!pageEventId || !pageId) return;
		onResolvedPage?.({ eventId: pageEventId, pageId });
	}, [onResolvedPage, pageEventId, pageId]);
	const isPagePending = Boolean(
		pageKey && !bootstrapPageData && resolvedPageKey !== pageKey,
	);
	// Auth/profile initialization can refresh the same route/event objects after
	// an early native page read failed. Use the query generation to retry even
	// when every page key field remains unchanged.
	const catalogDataUpdatedAt = Math.max(
		events.dataUpdatedAt,
		bootstrap.dataUpdatedAt,
	);

	// --- Pre-sync board for the active event ---
	// On fresh installs the board file may not exist locally yet.
	// Calling getBoard with forceFresh ensures it is fetched from remote and
	// persisted before the user triggers their first execution.
	useEffect(() => {
		const target = activeEvent;
		if (
			!appId ||
			!target?.board_id ||
			(target.default_page_id && shouldWaitForPageBoardSync)
		)
			return;
		const version = normalizeBoardVersion(target.board_version);
		void trackBoardReadiness(
			boardReadinessKey(appId, target.board_id, version),
			() => backend.boardState.getBoard(appId, target.board_id, version, true),
		);
	}, [appId, activeEvent, shouldWaitForPageBoardSync, backend.boardState]);

	// --- Event switching ---

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef is a stable ref
	const switchEvent = useCallback(
		(newEventId: string, replace = false) => {
			if (!appId || !newEventId || eventId === newEventId) return;
			headerRef.current?.pushToolbarElements([]);
			headerRef.current?.pushNavElements([]);
			if (embedded) {
				onNavigate?.({ eventId: newEventId });
				return;
			}
			setQueryParams("eventId", newEventId, { replace });
		},
		[appId, eventId, embedded, onNavigate, setQueryParams],
	);

	// --- Config ---

	const config = useMemo(() => {
		if (!activeEvent) return {};
		try {
			return parseUint8ArrayToJson(activeEvent.config) ?? {};
		} catch {
			return {};
		}
	}, [activeEvent]);

	// --- Page loading ---

	useEffect(() => {
		// These reads intentionally make a successful catalog refresh, or a manual
		// or scheduled retry, re-run a page lookup whose route/event identity did
		// not otherwise change.
		void catalogDataUpdatedAt;
		void pageRetry;
		if (!appId || !pageId) {
			setPageData(null);
			setPageError(null);
			setResolvedPageKey("");
			setPageLoading(false);
			return;
		}
		if (bootstrapPageData) {
			setPageData((current) =>
				isEqual(current, bootstrapPageData) ? current : bootstrapPageData,
			);
			setPageError(null);
			setResolvedPageKey(pageKey);
			setPageLoading(false);
			return;
		}
		if (supportsPageBootstrap) {
			setPageData(null);
			setPageError(null);
			setResolvedPageKey(pageKey);
			setPageLoading(false);
			return;
		}
		if (isRoutePending || routeLoading || isDirectEventPending) {
			return;
		}

		let cancelled = false;
		setPageLoading(true);
		setPageError(null);
		setPageData((currentPage) =>
			currentPage?.id === pageId ? currentPage : null,
		);

		// Identical content must keep its object identity: handing the interface a new object
		// for an unchanged page rebuilds its surface and throws away whatever it had rendered.
		const applyPage = (page: IPage) => {
			setPageData((current) => (isEqual(current, page) ? current : page));
			setPageError(null);
		};

		const loadPage = async () => {
			try {
				// A page one revision behind renders now and corrects itself when the refresh
				// lands, which beats holding a blank screen for a round trip. A pinned version
				// is immutable, so there is nothing to revalidate against.
				const readOptions: IGetPageOptions | undefined = pageBoardVersion
					? undefined
					: {
							revalidate: "background",
							onRevalidated: (fresh) => {
								if (!cancelled) applyPage(fresh);
							},
						};

				const page = shouldWaitForPageBoardSync
					? await loadPageWithBoardSync(
							backend.boardState,
							backend.pageState,
							appId,
							pageId,
							pageBoardId,
							pageBoardVersion,
							readOptions,
						)
					: await backend.pageState.getPage(
							appId,
							pageId,
							pageBoardId,
							pageBoardVersion,
							readOptions,
						);
				if (!cancelled) {
					applyPage(page);
				}
			} catch (e) {
				console.error("Failed to load page:", e);
				if (!cancelled) {
					setPageData(null);
					setPageError(pageLoadErrorMessage(e));
				}
			} finally {
				if (!cancelled) {
					setResolvedPageKey(pageKey);
					setPageLoading(false);
				}
			}
		};

		loadPage();
		return () => {
			cancelled = true;
		};
	}, [
		appId,
		pageId,
		bootstrapPageData,
		supportsPageBootstrap,
		pageBoardId,
		pageKey,
		isRoutePending,
		routeLoading,
		isDirectEventPending,
		pageBoardVersion,
		catalogDataUpdatedAt,
		pageRetry,
		shouldWaitForPageBoardSync,
		backend.boardState,
		backend.pageState,
	]);

	const pageRetryAttempt = pageRetry.key === pageKey ? pageRetry.attempt : 0;
	// Offline there is nothing to back off from: the connection coming back is the retry.
	const pageRetryPending =
		Boolean(pageError) &&
		isOnline &&
		pageRetryAttempt < PAGE_RETRY_DELAYS_MS.length;

	useEffect(() => {
		if (!pageKey || !pageRetryPending) return;
		const timer = setTimeout(() => {
			setPageRetry({ key: pageKey, attempt: pageRetryAttempt + 1 });
		}, PAGE_RETRY_DELAYS_MS[pageRetryAttempt]);
		return () => clearTimeout(timer);
	}, [pageKey, pageRetryPending, pageRetryAttempt]);

	const retryPage = useCallback(() => {
		setPageRetry({ key: pageKey, attempt: 0 });
	}, [pageKey]);

	const retryCatalog = useCallback(() => {
		if (bootstrapEnabled) void bootstrap.refetch();
		void events.refetch();
	}, [bootstrapEnabled, bootstrap.refetch, events.refetch]);

	const wasOnlineRef = useRef(isOnline);
	useEffect(() => {
		const reconnected = isOnline && !wasOnlineRef.current;
		wasOnlineRef.current = isOnline;
		if (!reconnected) return;
		if (pageKey && pageError) setPageRetry({ key: pageKey, attempt: 0 });
		if (!catalogEvents) retryCatalog();
	}, [isOnline, pageKey, pageError, catalogEvents, retryCatalog]);

	// A governed Page's action contract is derived from the whole Board, so an
	// edit made anywhere — the flow editor in another window, a collaborator, a
	// sync — supersedes what this mount is rendering, and its surface keeps
	// showing the pre-edit Page. The global query defaults disable focus
	// refetching, so nothing brings it forward on its own.
	const bootstrapRevalidatedAtRef = useRef(0);
	const revalidateBootstrap = useCallback(() => {
		if (!bootstrapEnabled) return;
		const now = Date.now();
		if (now - bootstrapRevalidatedAtRef.current < BOOTSTRAP_REVALIDATE_MIN_MS)
			return;
		bootstrapRevalidatedAtRef.current = now;
		void bootstrap.refetch();
	}, [bootstrapEnabled, bootstrap.refetch]);

	// Two triggers, one cleanup. Coming back to the foreground is the ordinary
	// case (edit the flow, switch back). The drift signal is the failure case: a
	// transport was refused for a reason only a fresh contract can cure — a
	// renamed action, a dead dynamic grant — so the surface it was dispatched
	// from is provably out of date. A successful run never signals: it has
	// already rewritten this surface through its own A2UI messages, and
	// refetching on top of that would re-run onLoad over live content.
	useEffect(() => {
		if (!bootstrapEnabled || typeof window === "undefined") return;
		const revalidate = () => {
			if (document.visibilityState === "hidden") return;
			revalidateBootstrap();
		};
		window.addEventListener("focus", revalidate);
		document.addEventListener("visibilitychange", revalidate);
		const unsubscribe = subscribeToPageContractDrift((detail) => {
			if (!isPageContractDriftFor(detail, appId, pageEventId)) return;
			revalidateBootstrap();
		});
		return () => {
			window.removeEventListener("focus", revalidate);
			document.removeEventListener("visibilitychange", revalidate);
			unsubscribe();
		};
	}, [bootstrapEnabled, appId, pageEventId, revalidateBootstrap]);

	// A restored event catalog can render a page before the session is: the query cache is
	// persisted, so a cold start replays the events while sign-in is still in flight, and every
	// read the page needs is refused for want of a token. The retry ladder is short and only a
	// reconnect re-arms it, so a sign-in that lands late used to leave a signed-in, online
	// device sitting on an error card. Signing in is a second chance, exactly like reconnecting.
	const hadAccessTokenRef = useRef(hasAccessToken);
	useEffect(() => {
		const signedIn = hasAccessToken && !hadAccessTokenRef.current;
		hadAccessTokenRef.current = hasAccessToken;
		if (!signedIn) return;
		if (pageKey && pageError) setPageRetry({ key: pageKey, attempt: 0 });
		if (!catalogEvents) retryCatalog();
	}, [hasAccessToken, pageKey, pageError, catalogEvents, retryCatalog]);

	// --- Route/event sync effects ---

	useEffect(() => {
		if (!effectiveRouteMapping) return;
		if (eventId && eventId !== effectiveRouteMapping.eventId) {
			if (embedded) {
				onNavigate?.({ eventId: null });
				return;
			}
			setQueryParams("eventId", undefined, { replace: true });
		}
	}, [effectiveRouteMapping, eventId, embedded, onNavigate, setQueryParams]);

	useEffect(() => {
		if (!appId) return;
		if (effectiveRouteMapping) return;
		if (routeLoading || isRoutePending || isDirectEventPending) return;

		const queriesPending = bootstrapPending || events.isFetching;

		if (sortedEvents.length === 0) {
			if (!catalogEvents || queriesPending) return;
			goToStore();
			return;
		}

		let rerouteEvent = sortedEvents.find((e) => canUseEvent(e));

		if (!rerouteEvent) {
			if (queriesPending) return;
			if (catalogEvents) goToStore();
			return;
		}

		const lastEventId = localStorage.getItem(`lastUsedEvent-${appId}`);
		const lastEvent = sortedEvents.find((e) => e.id === lastEventId);

		if (canUseEvent(lastEvent)) {
			rerouteEvent = lastEvent;
		}

		if (!resolvedCurrentEvent) {
			if (rerouteEvent) {
				switchEvent(rerouteEvent.id, true);
				return;
			}
			return;
		}

		if (eventId && !canUseEvent(resolvedCurrentEvent)) {
			switchEvent(rerouteEvent?.id ?? "", true);
			return;
		}

		localStorage.setItem(`lastUsedEvent-${appId}`, resolvedCurrentEvent.id);
	}, [
		appId,
		eventId,
		sortedEvents,
		resolvedCurrentEvent,
		switchEvent,
		canUseEvent,
		catalogEvents,
		events.isFetching,
		bootstrapPending,
		effectiveRouteMapping,
		routeLoading,
		isRoutePending,
		isDirectEventPending,
		goToStore,
	]);

	// --- Route navigation ---

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef is a stable ref
	const switchRoute = useCallback(
		(path: string) => {
			if (!appId || !path) return;
			headerRef.current?.pushToolbarElements([]);
			headerRef.current?.pushNavElements([]);
			if (embedded) {
				onNavigate?.({ routePath: path, eventId: null });
				return;
			}
			const params = new URLSearchParams(window.location.search);
			params.set("route", path);
			params.delete("eventId");
			router.push(`?${params.toString()}`);
		},
		[appId, embedded, onNavigate, router],
	);

	const handleEmbeddedNavigation = useCallback(
		(message: Parameters<typeof resolveEmbeddedPageNavigation>[0]) => {
			if (!appId || !embedded) return;
			const next = resolveEmbeddedPageNavigation(
				message,
				appId,
				queryParamsProp ?? {},
			);
			if (next.externalHref) {
				if (
					typeof window !== "undefined" &&
					isSafeEmbeddedExternalHref(next.externalHref, window.location.href)
				) {
					window.open(next.externalHref, "_blank", "noopener,noreferrer");
				}
				return;
			}
			onNavigate?.({
				...(next.routePath !== undefined ? { routePath: next.routePath } : {}),
				...(next.eventId !== undefined ? { eventId: next.eventId } : {}),
				queryParams: next.queryParams,
			});
		},
		[appId, embedded, onNavigate, queryParamsProp],
	);

	// --- Render logic ---

	// A silent token renewal or a background access re-check must not tear down an
	// interface that already resolved. It would remount the whole event tree and
	// look like the app reloading itself.
	const accessGateBlocking = Boolean(
		(redirectCheckPending || shouldRedirectToStore) && !activeEvent,
	);

	const shouldRenderHeader = useMemo(() => {
		if (accessGateBlocking) return false;
		if (routeLoading || isRoutePending || isDirectEventPending) return false;
		if (pageEvent?.default_page_id) return false;
		return true;
	}, [
		accessGateBlocking,
		routeLoading,
		isRoutePending,
		isDirectEventPending,
		pageEvent,
	]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef and sidebarRef are stable refs
	const inner = useMemo(() => {
		if (!appId) return notFound ?? <NoDefaultInterface appId="" />;
		if (
			accessGateBlocking ||
			routeLoading ||
			isRoutePending ||
			isDirectEventPending
		) {
			return <LoadingScreen />;
		}

		// Page-target event (from route or event fallback)
		if (pageEvent) {
			if (bootstrapPending) return <LoadingScreen />;
			if (pageExecutionAuthorityUnavailable) {
				return (
					<InterfaceLoadError
						message="This Page could not load its execution authorization. Reload and try again."
						offline={!isOnline}
						retrying={bootstrapPending}
						onRetry={retryCatalog}
					/>
				);
			}
			if (resolvedPageData && !isPagePending) {
				return (
					<div className="flex flex-col grow h-full w-full max-h-full overflow-hidden">
						<PageInterface
							key={`${pageKey}:${pageContentRevision ?? resolvedPageData.updatedAt}:${pageExecutionRevision ?? "unresolved"}`}
							appId={appId}
							event={pageEvent}
							config={parseUint8ArrayToJson(pageEvent.config) ?? {}}
							route={effectiveRouteMapping?.path ?? routePath}
							page={resolvedPageData}
							pageRevision={pageContentRevision}
							pageExecutionRevision={pageExecutionRevision}
							queryParams={embedded ? (queryParamsProp ?? {}) : undefined}
							active={active}
							onNavigationMessage={
								embedded ? handleEmbeddedNavigation : undefined
							}
							toolbarRef={headerRef}
							sidebarRef={sidebarRef}
						/>
					</div>
				);
			}
			if (pageLoading || isPagePending) return <LoadingScreen />;
			// The event does declare an interface, but it could not be read here. Saying
			// "no interface" would send the user to event configuration to fix data
			// that is already correct.
			if (pageError)
				return (
					<InterfaceLoadError
						message={pageError}
						offline={!isOnline}
						retrying={pageRetryPending}
						onRetry={retryPage}
					/>
				);
			return (
				<NoDefaultInterface appId={appId} eventId={eventId ?? undefined} />
			);
		}

		// Route targets an event (board/node interface)
		if (
			effectiveRouteEvent &&
			usableEvents.has(effectiveRouteEvent.event_type)
		) {
			const InterfaceComponent = usableEvents.get(
				effectiveRouteEvent.event_type,
			);
			if (InterfaceComponent) {
				return (
					<div
						key={effectiveRouteEvent.id}
						className="flex flex-col grow h-full w-full max-h-full overflow-hidden"
					>
						<InterfaceComponent
							appId={appId}
							event={effectiveRouteEvent}
							config={parseUint8ArrayToJson(effectiveRouteEvent.config) ?? {}}
							toolbarRef={headerRef}
							sidebarRef={sidebarRef}
						/>
					</div>
				);
			}
		}

		// No route config - fall back to event-based interface
		if (!activeEvent) {
			if ((bootstrapPending || events.isFetching) && !catalogEvents)
				return <LoadingScreen />;
			const hasUsableEvents = sortedEvents.some((e) => canUseEvent(e));
			if (hasUsableEvents && !eventId) return <LoadingScreen />;
			// Nothing cached and no way to fetch it. The app is not misconfigured, this
			// device simply has not seen its events yet.
			if (!catalogEvents && (events.isError || bootstrap.isError || !isOnline))
				return (
					<InterfaceLoadError
						message={
							isOnline
								? t(
										"thisAppsEventsCouldNotBeLoaded",
										"This app's events could not be loaded.",
									)
								: undefined
						}
						offline={!isOnline}
						retrying={bootstrapPending || events.isFetching}
						onRetry={retryCatalog}
					/>
				);
			return (
				<NoDefaultInterface appId={appId} eventId={eventId ?? undefined} />
			);
		}

		if (usableEvents.has(activeEvent.event_type)) {
			const InterfaceComponent = usableEvents.get(activeEvent.event_type);
			if (InterfaceComponent)
				return (
					<div
						key={activeEvent.id}
						className="flex flex-col grow h-full w-full max-h-full overflow-hidden"
					>
						<InterfaceComponent
							appId={appId}
							event={activeEvent}
							config={config}
							toolbarRef={headerRef}
							sidebarRef={sidebarRef}
						/>
					</div>
				);
		}

		return <NoDefaultInterface appId={appId} eventId={eventId ?? undefined} />;
	}, [
		appId,
		routeLoading,
		isRoutePending,
		isDirectEventPending,
		resolvedPageData,
		pageContentRevision,
		pageExecutionRevision,
		pageExecutionAuthorityUnavailable,
		pageLoading,
		pageError,
		pageRetryPending,
		retryPage,
		retryCatalog,
		isOnline,
		isPagePending,
		pageEvent,
		effectiveRouteMapping,
		routePath,
		embedded,
		queryParamsProp,
		active,
		handleEmbeddedNavigation,
		effectiveRouteEvent,
		sortedEvents,
		activeEvent,
		config,
		eventId,
		usableEvents,
		canUseEvent,
		events.isFetching,
		events.isError,
		bootstrap.isError,
		bootstrapPending,
		catalogEvents,
		notFound,
		accessGateBlocking,
	]);

	if (!appId) {
		return <>{notFound ?? <NoDefaultInterface appId="" />}</>;
	}

	const Root = embedded ? "div" : "main";
	return (
		<Root className="flex flex-col h-full overflow-hidden flex-1 min-h-0">
			<Container ref={sidebarRef}>
				<div className="flex flex-col grow h-full w-full max-h-full overflow-hidden">
					{shouldRenderHeader ? (
						<Header
							ref={headerRef}
							routes={availableRoutes}
							currentRoutePath={effectiveRouteMapping?.path ?? routePath}
							onNavigateRoute={switchRoute}
							usableEvents={new Set(usableEvents.keys())}
							currentEvent={activeEvent}
							sortedEvents={sortedEvents}
							metadata={metadata.data}
							appId={appId}
							switchEvent={switchEvent}
						/>
					) : null}
					{inner}
				</div>
			</Container>
		</Root>
	);
}
