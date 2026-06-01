"use client";

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
import type { IEvent } from "../../lib/schema/flow/event";
import { useSetQueryParams } from "../../lib/set-query-params";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { useBackend } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import type { ISettingsProfile } from "../../types";
import { LoadingScreen } from "../ui/loading-screen";
import { Container } from "./container";
import { Header } from "./header";
import type {
	IEventMapping,
	ISidebarActions,
	IToolBarActions,
	IUseInterfaceProps,
} from "./interfaces";
import { NoDefaultInterface } from "./no-default";
import { PageInterface } from "./page-interface";

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
	const hasAccessToken = Boolean(auth.user?.access_token);

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

	const localProfiles = useInvoke(
		backend.userState.getAllSettingsProfiles,
		backend.userState,
		[],
		Boolean(appId && !embedded && !auth.isLoading && !hasAccessToken),
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

	const needsUnauthenticatedLocalCheck = Boolean(
		appId && !embedded && !auth.isLoading && !hasAccessToken,
	);
	const localProfileCheckPending =
		needsUnauthenticatedLocalCheck &&
		localProfiles.isFetching &&
		!localProfiles.data &&
		!localProfiles.isError;
	const shouldRedirectUnauthenticatedToStore = Boolean(
		needsUnauthenticatedLocalCheck &&
			!localProfileCheckPending &&
			!appIsInAnyLocalProfile,
	);

	const needsAuthenticatedRemoteCheck = Boolean(
		appId && !embedded && hasAccessToken,
	);
	const authenticatedRemoteCheckPending =
		needsAuthenticatedRemoteCheck &&
		remoteApp.isFetching &&
		!remoteApp.data &&
		!remoteApp.isError;
	const shouldRedirectAuthenticatedToStore = Boolean(
		needsAuthenticatedRemoteCheck &&
			!authenticatedRemoteCheckPending &&
			!appIsInAnyLocalProfile &&
			!remoteApp.data &&
			remoteApp.isError,
	);

	const shouldRedirectFetchErrorToStore = Boolean(
		appId && !embedded && (routes.isError || events.isError),
	);

	const redirectCheckPending = Boolean(
		authCheckPending ||
			localProfileCheckPending ||
			authenticatedRemoteCheckPending,
	);
	const shouldRedirectToStore = Boolean(
		shouldRedirectUnauthenticatedToStore ||
			shouldRedirectAuthenticatedToStore ||
			shouldRedirectFetchErrorToStore,
	);

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
		if (
			!appId ||
			embedded ||
			redirectCheckPending ||
			!shouldRedirectToStore
		) {
			return;
		}

		router.replace(storeHref);
	}, [
		appId,
		embedded,
		redirectCheckPending,
		shouldRedirectToStore,
		router,
		storeHref,
	]);

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

	const pageKey =
		appId && pageEventId && pageId
			? `${appId}:${pageEventId}:${pageId}:${pageBoardId ?? ""}`
			: "";
	const isPagePending = Boolean(pageKey && resolvedPageKey !== pageKey);

	// --- Pre-sync board for the active event ---
	// On fresh installs the board file may not exist locally yet.
	// Calling getBoard with forceFresh ensures it is fetched from remote and
	// persisted before the user triggers their first execution.
	useEffect(() => {
		const target = activeEvent;
		if (!appId || !target?.board_id) return;
		backend.boardState
			.getBoard(appId, target.board_id, undefined, true)
			.catch(() => {});
	}, [appId, activeEvent, backend.boardState]);

	// --- Event switching ---

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef is a stable ref
	const switchEvent = useCallback(
		(newEventId: string) => {
			if (!appId || !newEventId || eventId === newEventId) return;
			headerRef.current?.pushToolbarElements([]);
			headerRef.current?.pushNavElements([]);
			if (embedded) {
				onNavigate?.({ eventId: newEventId });
				return;
			}
			setQueryParams("eventId", newEventId);
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
		if (!appId || !pageId) {
			setPageData(null);
			setResolvedPageKey("");
			setPageLoading(false);
			return;
		}
		if (isRoutePending || routeLoading || isDirectEventPending) {
			return;
		}

		let cancelled = false;
		setPageLoading(true);
		setPageData((currentPage) =>
			currentPage?.id === pageId ? currentPage : null,
		);

		const loadPage = async () => {
			try {
				const page = await backend.pageState.getPage(
					appId,
					pageId,
					pageBoardId,
				);
				if (!cancelled) {
					setPageData(page);
				}
			} catch (e) {
				console.error("Failed to load page:", e);
				if (!cancelled) {
					setPageData(null);
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
		backend.pageState,
	]);

	// --- Route/event sync effects ---

	useEffect(() => {
		if (!effectiveRouteMapping) return;
		if (eventId && eventId !== effectiveRouteMapping.eventId) {
			if (embedded) {
				onNavigate?.({ eventId: null });
				return;
			}
			setQueryParams("eventId", undefined);
		}
	}, [effectiveRouteMapping, eventId, embedded, onNavigate, setQueryParams]);

	useEffect(() => {
		if (!appId) return;
		if (effectiveRouteMapping) return;
		if (routeLoading || isRoutePending || isDirectEventPending) return;

		const queriesPending = routes.isFetching || events.isFetching;

		if (sortedEvents.length === 0) {
			if (!events.data || queriesPending) return;
			if (embedded) return;
			router.replace(storeHref);
			return;
		}

		let rerouteEvent = sortedEvents.find((e) => canUseEvent(e));

		if (!rerouteEvent) {
			if (queriesPending) return;
			if (events.data) {
				if (embedded) return;
				router.replace(storeHref);
			}
			return;
		}

		const lastEventId = localStorage.getItem(`lastUsedEvent-${appId}`);
		const lastEvent = sortedEvents.find((e) => e.id === lastEventId);

		if (canUseEvent(lastEvent)) {
			rerouteEvent = lastEvent;
		}

		if (!resolvedCurrentEvent) {
			if (rerouteEvent) {
				switchEvent(rerouteEvent.id);
				return;
			}
			return;
		}

		if (eventId && !canUseEvent(resolvedCurrentEvent)) {
			switchEvent(rerouteEvent?.id ?? "");
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
		router,
		embedded,
		storeHref,
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

	const shouldRenderHeader = useMemo(() => {
		if (redirectCheckPending) return false;
		if (shouldRedirectToStore) return false;
		if (routeLoading || isRoutePending || isDirectEventPending) return false;
		if (pageEvent?.default_page_id) return false;
		return true;
	}, [
		redirectCheckPending,
		shouldRedirectToStore,
		routeLoading,
		isRoutePending,
		isDirectEventPending,
		pageEvent,
	]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef and sidebarRef are stable refs
	const inner = useMemo(() => {
		if (!appId) return notFound ?? <NoDefaultInterface appId="" />;
		if (
			redirectCheckPending ||
			shouldRedirectToStore ||
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
		redirectCheckPending,
		shouldRedirectToStore,
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
