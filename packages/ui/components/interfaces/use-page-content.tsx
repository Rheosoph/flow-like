"use client";

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
import { normalizeBoardVersion } from "../../lib/schema/flow/board-version";
import type { IEvent } from "../../lib/schema/flow/event";
import { useSetQueryParams } from "../../lib/set-query-params";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { useBackend } from "../../state/backend-state";
import type { IBoardState } from "../../state/backend-state/board-state";
import type { IPage, IPageState } from "../../state/backend-state/page-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import type { ISettingsProfile } from "../../types";
import { LoadingScreen } from "../ui/loading-screen";
import { Container } from "./container";
import { Header } from "./header";
import { InterfaceLoadError } from "./interface-load-error";
import type {
	IEventMapping,
	ISidebarActions,
	IToolBarActions,
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

export function pageLoadErrorMessage(error: unknown): string {
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

export interface UsePageContentProps {
	eventConfig: IEventMapping;
	notFound?: ReactNode;
	appId?: string | null;
	routePath?: string | null;
	eventId?: string | null;
	embedded?: boolean;
	onNavigate?: (next: {
		routePath?: string | null;
		eventId?: string | null;
	}) => void;
}

/**
 * Page files are indexed through their board on native clients. Wait for the
 * existing force-fresh board sync before asking the native backend for the
 * page so a fresh install cannot observe the board halfway through syncing.
 *
 * A failed board sync must not prevent page-state's own local/remote fallback
 * from running (and web pages do not require a local board at all).
 *
 * An event pinned to a board version reads that version's published page: the
 * current page file belongs to the draft board and may already have moved on.
 */
export async function loadPageAfterBoardSync(
	boardState: Pick<IBoardState, "getBoard">,
	pageState: Pick<IPageState, "getPage">,
	appId: string,
	pageId: string,
	boardId?: string,
	boardVersion?: [number, number, number],
): Promise<IPage> {
	if (boardId) {
		await boardState
			.getBoard(appId, boardId, boardVersion, true)
			.catch(() => undefined);
	}

	return pageState.getPage(appId, pageId, boardId, boardVersion);
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
 * interface — routes in particular are optional metadata whose absence simply
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

export function UsePageContent({
	eventConfig,
	notFound,
	appId: appIdProp,
	routePath: routePathProp,
	eventId: eventIdProp,
	embedded = false,
	onNavigate,
}: Readonly<UsePageContentProps>) {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const router = useRouter();
	const auth = useAuth();
	const isOnline = useNetworkStatus();
	const hasAccessToken = Boolean(auth.user?.access_token);
	const shouldWaitForPageBoardSync = backend.capabilities().canExecuteLocally;

	const appId = appIdProp ?? searchParams.get("id");
	const routePath = routePathProp ?? searchParams.get("route") ?? "/";
	const eventId = eventIdProp ?? searchParams.get("eventId");
	const authCheckPending = Boolean(appId && !embedded && auth.isLoading);

	const headerRef = useRef<IToolBarActions>(
		null,
	) as React.RefObject<IToolBarActions>;
	const sidebarRef = useRef<ISidebarActions>(
		null,
	) as React.RefObject<ISidebarActions>;
	const setQueryParams = useSetQueryParams();

	// --- Data fetching (force-fresh with offline fallback) ---

	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId ?? "", true],
		typeof appId === "string",
	);

	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId ?? "", true],
		(appId ?? "") !== "",
		[],
	);

	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId ?? ""],
		typeof appId === "string",
		[],
	);

	// Signed-in users open locally installed apps too, so the local profiles
	// stay authoritative for access even when a hub lookup is available.
	const needsLocalProfileCheck = Boolean(appId && !embedded && !auth.isLoading);

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
		Boolean(appId && !embedded && hasAccessToken),
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
		needsLocalProfileCheck &&
		localProfiles.isFetching &&
		!localProfiles.data &&
		!localProfiles.isError;

	const needsAuthenticatedRemoteCheck = Boolean(
		appId && !embedded && hasAccessToken,
	);
	const authenticatedRemoteCheckPending =
		needsAuthenticatedRemoteCheck &&
		remoteApp.isFetching &&
		!remoteApp.data &&
		!remoteApp.isError;

	const storeRedirect = useMemo(
		() =>
			resolveStoreRedirect({
				embedded: embedded || !appId,
				authLoading: authCheckPending,
				hasAccessToken,
				appInLocalProfile: appIsInAnyLocalProfile,
				localProfileCheckPending,
				remoteAppCheckPending: authenticatedRemoteCheckPending,
				remoteAppLoaded: Boolean(remoteApp.data),
				remoteAppFailed: remoteApp.isError,
				eventsLoaded: Boolean(events.data),
				eventsFailed: events.isError,
				eventsFetching: events.isFetching,
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
			events.data,
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

	const sortedEvents = useMemo(() => {
		if (!events.data) return [];
		return events.data
			.filter((a) => a.active)
			.toSorted((a, b) => a.priority - b.priority);
	}, [events.data]);

	const currentEvent = useMemo(() => {
		if (!eventId) return undefined;
		return sortedEvents.find((e) => e.id === eventId);
	}, [eventId, sortedEvents]);

	const canUseEvent = useCallback(
		(event: IEvent | null | undefined) =>
			Boolean(
				event && (event.default_page_id || usableEvents.has(event.event_type)),
			),
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

		if (routes.isFetching && !routes.data) {
			if (isNavigation || needsFreshResolution) {
				setRouteLoading(true);
			}
			return;
		}

		const availableRoutes = routes.data ?? [];
		const defaultRoute = availableRoutes.find((r) => r.path === "/") ?? null;

		let mapping: IRouteMapping | null =
			routePath && routePath !== "/"
				? (availableRoutes.find((r) => r.path === routePath) ?? null)
				: defaultRoute;

		if (!mapping) {
			mapping = defaultRoute;
		}

		const cachedRouteEvent = mapping
			? (events.data?.find((event) => event.id === mapping.eventId) ?? null)
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
		routes.data,
		routes.isFetching,
		events.data,
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
		return canUseEvent(routeEvent) ? routeEvent : null;
	}, [canUseEvent, routeEvent]);

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
	const isPagePending = Boolean(pageKey && resolvedPageKey !== pageKey);
	// Auth/profile initialization can refresh the same route/event objects after
	// an early native page read failed. Use the query generation to retry even
	// when every page key field remains unchanged.
	const catalogDataUpdatedAt = Math.max(
		routes.dataUpdatedAt,
		events.dataUpdatedAt,
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
		backend.boardState
			.getBoard(
				appId,
				target.board_id,
				normalizeBoardVersion(target.board_version),
				true,
			)
			.catch(() => {});
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
		if (isRoutePending || routeLoading || isDirectEventPending) {
			return;
		}

		let cancelled = false;
		setPageLoading(true);
		setPageError(null);
		setPageData((currentPage) =>
			currentPage?.id === pageId ? currentPage : null,
		);

		const loadPage = async () => {
			try {
				const page = shouldWaitForPageBoardSync
					? await loadPageAfterBoardSync(
							backend.boardState,
							backend.pageState,
							appId,
							pageId,
							pageBoardId,
							pageBoardVersion,
						)
					: await backend.pageState.getPage(
							appId,
							pageId,
							pageBoardId,
							pageBoardVersion,
						);
				if (!cancelled) {
					// A catalog refresh re-reads the same page. Handing the interface a
					// new object identity for unchanged content rebuilds its surface and
					// throws away whatever the running page had rendered.
					setPageData((current) => (isEqual(current, page) ? current : page));
					setPageError(null);
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
		void routes.refetch();
		void events.refetch();
	}, [routes.refetch, events.refetch]);

	const wasOnlineRef = useRef(isOnline);
	useEffect(() => {
		const reconnected = isOnline && !wasOnlineRef.current;
		wasOnlineRef.current = isOnline;
		if (!reconnected) return;
		if (pageKey && pageError) setPageRetry({ key: pageKey, attempt: 0 });
		if (!events.data) retryCatalog();
	}, [isOnline, pageKey, pageError, events.data, retryCatalog]);

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

		const queriesPending = routes.isFetching || events.isFetching;

		if (sortedEvents.length === 0) {
			if (!events.data || queriesPending) return;
			goToStore();
			return;
		}

		let rerouteEvent = sortedEvents.find((e) => canUseEvent(e));

		if (!rerouteEvent) {
			if (queriesPending) return;
			if (events.data) goToStore();
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
		events.data,
		events.isFetching,
		effectiveRouteMapping,
		routeLoading,
		isRoutePending,
		isDirectEventPending,
		routes.isFetching,
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

	// --- Render logic ---

	// A silent token renewal or a background access re-check must not tear down an
	// interface that already resolved — it would remount the whole event tree and
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
			if (pageData && !isPagePending) {
				return (
					<div className="flex flex-col grow h-full w-full max-h-full overflow-hidden">
						<PageInterface
							appId={appId}
							event={pageEvent}
							config={parseUint8ArrayToJson(pageEvent.config) ?? {}}
							page={pageData}
							toolbarRef={headerRef}
							sidebarRef={sidebarRef}
						/>
					</div>
				);
			}
			if (pageLoading || isPagePending) return <LoadingScreen />;
			// The event does declare an interface — it could not be read here. Saying
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
			if (events.isFetching && !events.data) return <LoadingScreen />;
			const hasUsableEvents = sortedEvents.some((e) => canUseEvent(e));
			if (hasUsableEvents && !eventId) return <LoadingScreen />;
			// Nothing cached and no way to fetch it. The app is not misconfigured, this
			// device simply has not seen its events yet.
			if (!events.data && (events.isError || !isOnline))
				return (
					<InterfaceLoadError
						message={
							isOnline ? "This app's events could not be loaded." : undefined
						}
						offline={!isOnline}
						retrying={events.isFetching}
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
		pageData,
		pageLoading,
		pageError,
		pageRetryPending,
		retryPage,
		retryCatalog,
		isOnline,
		isPagePending,
		pageEvent,
		effectiveRouteEvent,
		sortedEvents,
		activeEvent,
		config,
		eventId,
		usableEvents,
		canUseEvent,
		events.isFetching,
		events.data,
		notFound,
		accessGateBlocking,
	]);

	if (!appId) {
		return <>{notFound ?? <NoDefaultInterface appId="" />}</>;
	}

	return (
		<main className="flex flex-col h-full overflow-hidden flex-1 min-h-0">
			<Container ref={sidebarRef}>
				<div className="flex flex-col grow h-full w-full max-h-full overflow-hidden">
					{shouldRenderHeader ? (
						<Header
							ref={headerRef}
							routes={routes.data ?? []}
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
		</main>
	);
}
