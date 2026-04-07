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
import { useInvoke } from "../../hooks/use-invoke";
import type { IEvent } from "../../lib/schema/flow/event";
import { useSetQueryParams } from "../../lib/set-query-params";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { useBackend } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
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
}

export function UsePageContent({
	eventConfig,
	notFound,
}: Readonly<UsePageContentProps>) {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const router = useRouter();

	const appId = searchParams.get("id");
	const routePath = searchParams.get("route") ?? "/";
	const eventId = searchParams.get("eventId");

	const headerRef = useRef<IToolBarActions>(
		null,
	) as React.RefObject<IToolBarActions>;
	const sidebarRef = useRef<ISidebarActions>(
		null,
	) as React.RefObject<ISidebarActions>;
	const setQueryParams = useSetQueryParams();

	// --- Data fetching (always force-refresh on the /use page) ---

	const getRoutesForced = useMemo(() => {
		const getRoutes = (currentAppId: string) =>
			backend.routeState.getRoutes(currentAppId, true);
		return getRoutes;
	}, [backend.routeState]);

	const routes = useInvoke(
		getRoutesForced,
		backend.routeState,
		[appId ?? ""],
		typeof appId === "string",
		[appId],
	);

	const getEventsForced = useMemo(() => {
		const getEvents = (currentAppId: string) =>
			backend.eventState.getEvents(currentAppId, true);
		return getEvents;
	}, [backend.eventState]);

	const events = useInvoke(
		getEventsForced,
		backend.eventState,
		[appId ?? ""],
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

	// --- Route & event resolution ---

	const [routeMapping, setRouteMapping] = useState<IRouteMapping | null>(null);
	const [routeEvent, setRouteEvent] = useState<IEvent | null>(null);
	const [pageData, setPageData] = useState<IPage | null>(null);
	const [routeLoading, setRouteLoading] = useState(true);
	const [pageLoading, setPageLoading] = useState(false);

	const resolveKeyRef = useRef("");

	useEffect(() => {
		if (!appId) {
			setRouteMapping(null);
			setRouteEvent(null);
			setPageData(null);
			setRouteLoading(false);
			return;
		}

		// Wait for route data before resolving
		if (routes.isFetching && !routes.data) {
			setRouteLoading(true);
			return;
		}

		const currentKey = `${appId}:${routePath}`;
		const isNavigation = resolveKeyRef.current !== currentKey;
		resolveKeyRef.current = currentKey;

		// Only clear old state on actual navigation, not on data refreshes
		if (isNavigation) {
			setRouteMapping(null);
			setRouteEvent(null);
			setPageData(null);
		}

		let cancelled = false;
		setRouteLoading(true);

		const resolve = async () => {
			const availableRoutes = routes.data ?? [];
			const defaultRoute = availableRoutes.find((r) => r.path === "/") ?? null;

			let mapping: IRouteMapping | null =
				routePath && routePath !== "/"
					? (availableRoutes.find((r) => r.path === routePath) ?? null)
					: defaultRoute;

			if (!mapping) mapping = defaultRoute;
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
				if (!cancelled) setRouteLoading(false);
			}
		};

		resolve();
		return () => {
			cancelled = true;
		};
	}, [appId, routePath, routes.data, routes.isFetching, backend.eventState]);

	// --- Active event ---

	const activeEvent = useMemo(() => {
		if (routeEvent) return routeEvent;
		return currentEvent;
	}, [routeEvent, currentEvent]);

	// --- Pre-sync board for the active event ---
	// On fresh installs the board file may not exist locally yet.
	// Calling getBoard with forceFresh ensures it is fetched from remote and
	// persisted before the user triggers their first execution.
	useEffect(() => {
		const target = routeEvent ?? activeEvent;
		if (!appId || !target?.board_id) return;
		backend.boardState
			.getBoard(appId, target.board_id, undefined, true)
			.catch(() => {});
	}, [appId, routeEvent, activeEvent, backend.boardState]);

	// --- Event switching ---

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef is a stable ref
	const switchEvent = useCallback(
		(newEventId: string) => {
			if (!appId || !newEventId || eventId === newEventId) return;
			headerRef.current?.pushToolbarElements([]);
			headerRef.current?.pushNavElements([]);
			setQueryParams("eventId", newEventId);
		},
		[appId, eventId, setQueryParams],
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
		if (!appId) {
			setPageData(null);
			setPageLoading(false);
			return;
		}
		if (routeLoading) return;

		const targetEvent = routeEvent ?? activeEvent;
		if (!targetEvent) {
			setPageData(null);
			setPageLoading(false);
			return;
		}

		let cancelled = false;
		setPageLoading(true);

		const loadPage = async () => {
			try {
				if (targetEvent.default_page_id) {
					const page = await backend.pageState.getPage(
						appId,
						targetEvent.default_page_id,
						targetEvent.board_id || undefined,
					);
					if (!cancelled) {
						setPageData(page);
						setPageLoading(false);
					}
					return;
				}

				if (!cancelled) {
					setPageData(null);
					setPageLoading(false);
				}
			} catch (e) {
				console.error("Failed to load page:", e);
				if (!cancelled) {
					setPageData(null);
					setPageLoading(false);
				}
			}
		};

		loadPage();
		return () => {
			cancelled = true;
		};
	}, [appId, routeLoading, routeEvent, activeEvent, backend.pageState]);

	// --- Route/event sync effects ---

	useEffect(() => {
		if (!routeMapping) return;
		if (eventId && eventId !== routeMapping.eventId) {
			setQueryParams("eventId", undefined);
		}
	}, [routeMapping, eventId, setQueryParams]);

	useEffect(() => {
		if (!appId) return;
		if (routeMapping) return;
		if (routeLoading) return;
		if ((routes.data?.length ?? 0) > 0 && routeEvent) return;

		const queriesPending = routes.isFetching || events.isFetching;

		if (sortedEvents.length === 0) {
			if (!events.data || queriesPending) return;
			router.replace(`/store?id=${appId}`);
			return;
		}

		let rerouteEvent = sortedEvents.find(
			(e) => usableEvents.has(e.event_type) || e.default_page_id,
		);

		if (!rerouteEvent) {
			if (queriesPending) return;
			if (events.data) {
				router.replace(`/store?id=${appId}`);
			}
			return;
		}

		const lastEventId = localStorage.getItem(`lastUsedEvent-${appId}`);
		const lastEvent = sortedEvents.find((e) => e.id === lastEventId);

		if (
			lastEvent &&
			(usableEvents.has(lastEvent.event_type) || lastEvent.default_page_id)
		) {
			rerouteEvent = lastEvent;
		}

		if (!currentEvent) {
			if (rerouteEvent) {
				switchEvent(rerouteEvent.id);
				return;
			}
			return;
		}

		if (
			eventId &&
			!usableEvents.has(currentEvent.event_type) &&
			!currentEvent.default_page_id
		) {
			switchEvent(rerouteEvent?.id ?? "");
			return;
		}

		localStorage.setItem(`lastUsedEvent-${appId}`, eventId ?? "");
	}, [
		appId,
		eventId,
		sortedEvents,
		currentEvent,
		switchEvent,
		usableEvents,
		events.data,
		events.isFetching,
		routeMapping,
		routeLoading,
		routes.data,
		routes.isFetching,
		router,
		routeEvent,
	]);

	// --- Route navigation ---

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef is a stable ref
	const switchRoute = useCallback(
		(path: string) => {
			if (!appId || !path) return;
			headerRef.current?.pushToolbarElements([]);
			headerRef.current?.pushNavElements([]);
			const params = new URLSearchParams(window.location.search);
			params.set("route", path);
			params.delete("eventId");
			router.push(`?${params.toString()}`);
		},
		[appId, router],
	);

	// --- Render logic ---

	const shouldRenderHeader = useMemo(() => {
		if (routeLoading) return false;
		if (routeEvent?.default_page_id) return false;
		if (pageData && activeEvent?.default_page_id) return false;
		return true;
	}, [routeLoading, routeEvent, activeEvent, pageData]);

	const isResolvingCurrentRoute = useMemo(() => {
		if (routeLoading) return true;
		if (!appId) return false;
		if (routePath === "/") return false;
		if (routeMapping) return false;
		return routes.isFetching;
	}, [appId, routeLoading, routeMapping, routePath, routes.isFetching]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: headerRef and sidebarRef are stable refs
	const inner = useMemo(() => {
		if (!appId) return notFound ?? <NoDefaultInterface appId="" />;
		if (isResolvingCurrentRoute) return <LoadingScreen />;

		// Page-target event (from route or event fallback)
		const pageEvent = routeEvent?.default_page_id
			? routeEvent
			: activeEvent?.default_page_id
				? activeEvent
				: null;
		if (pageEvent) {
			if (pageData) {
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
			if (pageLoading) return <LoadingScreen />;
			return (
				<NoDefaultInterface appId={appId} eventId={eventId ?? undefined} />
			);
		}

		// Route targets an event (board/node interface)
		if (routeEvent && usableEvents.has(routeEvent.event_type)) {
			const InterfaceComponent = usableEvents.get(routeEvent.event_type);
			if (InterfaceComponent) {
				return (
					<div
						key={routeEvent.id}
						className="flex flex-col grow h-full w-full max-h-full overflow-hidden"
					>
						<InterfaceComponent
							appId={appId}
							event={routeEvent}
							config={parseUint8ArrayToJson(routeEvent.config) ?? {}}
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
			const hasUsableEvents = sortedEvents.some(
				(e) => usableEvents.has(e.event_type) || e.default_page_id,
			);
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
		isResolvingCurrentRoute,
		pageData,
		pageLoading,
		routeEvent,
		sortedEvents,
		activeEvent,
		config,
		eventId,
		usableEvents,
		events.isFetching,
		events.data,
		notFound,
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
							currentRoutePath={routeMapping?.path ?? routePath}
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
