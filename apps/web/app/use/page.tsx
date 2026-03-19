"use client";

import {
	Container,
	Header,
	type IToolBarActions,
	LoadingScreen,
	NoDefaultInterface,
	PageInterface,
	useBackend,
	useInvoke,
	useSetQueryParams,
} from "@tm9657/flow-like-ui";
import type {
	ISidebarActions,
	IUseInterfaceProps,
} from "@tm9657/flow-like-ui/components/interfaces/interfaces";
import type { IEvent } from "@tm9657/flow-like-ui/lib/schema/flow/event";
import { parseUint8ArrayToJson } from "@tm9657/flow-like-ui/lib/uint8";
import type { IPage } from "@tm9657/flow-like-ui/state/backend-state/page-state";
import type { IRouteMapping } from "@tm9657/flow-like-ui/state/backend-state/route-state";
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
import { EVENT_CONFIG } from "../../lib/event-config";
import NotFound from "../library/config/not-found";

export default function UsePage() {
	const backend = useBackend();
	const searchParams = useSearchParams();
	const router = useRouter();

	const appId = searchParams.get("id");
	const routePath = searchParams.get("route") ?? "/";

	const headerRef = useRef<IToolBarActions>(null!);
	const sidebarRef = useRef<ISidebarActions>(null!);
	const setQueryParams = useSetQueryParams();

	const routes = useInvoke(
		useMemo(() => {
			const getRoutes = (currentAppId: string) =>
				backend.routeState.getRoutes(currentAppId, true);
			return getRoutes;
		}, [backend.routeState]),
		backend.routeState,
		[appId ?? ""],
		typeof appId === "string",
		[appId],
	);

	// Route and page state
	const [routeMapping, setRouteMapping] = useState<IRouteMapping | null>(null);
	const [routeEvent, setRouteEvent] = useState<IEvent | null>(null);
	const [pageData, setPageData] = useState<IPage | null>(null);
	const [routeLoading, setRouteLoading] = useState(true);

	// Load route config and associated event
	useEffect(() => {
		if (!appId) {
			setRouteMapping(null);
			setRouteEvent(null);
			setPageData(null);
			setRouteLoading(false);
			return;
		}

		const loadRoute = async () => {
			setRouteMapping(null);
			setRouteEvent(null);
			setPageData(null);
			setRouteLoading(true);
			try {
				const availableRoutes = routes.data ?? [];
				const defaultRoute =
					availableRoutes.find((route) => route.path === "/") ?? null;

				let mapping: IRouteMapping | null =
					routePath && routePath !== "/"
						? availableRoutes.find((route) => route.path === routePath) ?? null
						: defaultRoute;

				if (!mapping && routePath !== "/" && routes.isFetching) {
					return;
				}

				if (!mapping) {
					mapping = defaultRoute;
				}

				setRouteMapping(mapping);

				// Load the event if we have a mapping
				if (mapping) {
					const event = await backend.eventState.getEvent(
						appId,
						mapping.eventId,
					);
					setRouteEvent(event);
				} else {
					setRouteEvent(null);
				}
			} catch (e) {
				console.error("Failed to load route:", e);
				setRouteMapping(null);
				setRouteEvent(null);
			} finally {
				setRouteLoading(false);
			}
		};

		loadRoute();
	}, [appId, routePath, routes.data, routes.isFetching, backend.eventState]);

	const usableEvents = useMemo(() => {
		const events = new Map<
			string,
			(props: IUseInterfaceProps) => JSX.Element | ReactNode | null
		>();
		Object.values(EVENT_CONFIG).forEach((config) => {
			const usable = Object.entries(config.useInterfaces);
			for (const [eventType, useInterface] of usable) {
				if (config.eventTypes.includes(eventType)) {
					events.set(eventType, useInterface);
				}
			}
		});
		return events;
	}, []);

	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[appId ?? ""],
		typeof appId === "string",
	);

	const eventId = searchParams.get("eventId");
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId ?? ""],
		(appId ?? "") !== "",
	);
	const sortedEvents = useMemo(() => {
		if (!events.data) return [];
		return events.data
			.filter((a) => a.active)
			.toSorted((a, b) => a.priority - b.priority);
	}, [events.data]);

	// Load page data for page-target events
	useEffect(() => {
		if (!appId) {
			setPageData(null);
			return;
		}
		if (routeLoading) return;
		if (!routeEvent) {
			setPageData(null);
			return;
		}

		let cancelled = false;
		const loadPage = async () => {
			try {
				// Event has default_page_id -> page-target event
				if (routeEvent.default_page_id) {
					const page = await backend.pageState.getPage(
						appId,
						routeEvent.default_page_id,
						routeEvent.board_id || undefined,
					);
					if (!cancelled) setPageData(page);
					return;
				}

				// Board-target event - no page
				if (!cancelled) setPageData(null);
			} catch (e) {
				console.error("Failed to load page:", e);
				if (!cancelled) setPageData(null);
			}
		};

		loadPage();
		return () => {
			cancelled = true;
		};
	}, [appId, routeLoading, routeEvent, backend.pageState]);

	const currentEvent = useMemo(() => {
		if (!eventId) return undefined;
		return sortedEvents.find((e) => e.id === eventId);
	}, [eventId, sortedEvents]);

	const activeEvent = useMemo(() => {
		// If we have a route event from the mapping, use that
		if (routeEvent) {
			return routeEvent;
		}
		// Fallback to current event from query params
		return currentEvent;
	}, [routeEvent, currentEvent]);

	const switchEvent = useCallback(
		(newEventId: string) => {
			if (!appId) return;
			if (!newEventId) return;
			if (eventId === newEventId) return;
			if (newEventId === "") return;

			headerRef.current?.pushToolbarElements([]);
			setQueryParams("eventId", newEventId);
		},
		[appId, eventId, setQueryParams],
	);

	const config = useMemo(() => {
		if (!activeEvent) return {};
		try {
			return parseUint8ArrayToJson(activeEvent.config) ?? {};
		} catch (e) {
			console.error("Error parsing event config:", e);
			return {};
		}
	}, [activeEvent]);

	useEffect(() => {
		if (!routeMapping) return;

		// Route navigation is path-based; avoid stale eventId params.
		if (eventId && eventId !== routeMapping.eventId) {
			setQueryParams("eventId", undefined);
		}
	}, [routeMapping, eventId, setQueryParams]);

	useEffect(() => {
		if (!appId) return;
		if (routeMapping) return;
		if (routeLoading) return;
		if ((routes.data?.length ?? 0) > 0) return;
		const queriesPending = routes.isFetching || events.isFetching;

		if (sortedEvents.length === 0) {
			if (!events.data || queriesPending) return;
			router.replace(`/store?id=${appId}`);
			return;
		}

		let rerouteEvent = sortedEvents.find((e) => usableEvents.has(e.event_type));

		if (!rerouteEvent) {
			if (queriesPending) return;
			if (events.data) {
				router.replace(`/store?id=${appId}`);
			}
			return;
		}

		const lastEventId = localStorage.getItem(`lastUsedEvent-${appId}`);
		const lastEvent = sortedEvents.find((e) => e.id === lastEventId);

		if (lastEvent && usableEvents.has(lastEvent.event_type)) {
			rerouteEvent = lastEvent;
		}

		if (!currentEvent) {
			if (rerouteEvent) {
				switchEvent(rerouteEvent.id);
				return;
			}
			return;
		}

		if (eventId && !usableEvents.has(currentEvent.event_type)) {
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
	]);

	const switchRoute = useCallback(
		(path: string) => {
			if (!appId) return;
			if (!path) return;

			headerRef.current?.pushToolbarElements([]);
			const params = new URLSearchParams(window.location.search);
			params.set("route", path);
			params.delete("eventId");
			router.push(`?${params.toString()}`);
		},
		[appId, router],
	);

	const shouldRenderHeader = useMemo(() => {
		if (routeLoading) return false;
		// Hide header if route event has a default page (full page rendering)
		if (routeEvent?.default_page_id) return false;
		return true;
	}, [routeLoading, routeEvent]);

	const isResolvingCurrentRoute = useMemo(() => {
		if (routeLoading) return true;
		if (!appId) return false;
		if (routePath === "/") return false;
		if (routeMapping) return false;
		return routes.isFetching;
	}, [appId, routeLoading, routeMapping, routePath, routes.isFetching]);

	const inner = useMemo(() => {
		if (!appId) return <NotFound />;
		if (isResolvingCurrentRoute) return <LoadingScreen />;

		// Route event has a page - render the page interface
		if (pageData && routeEvent?.default_page_id) {
			return (
				<div className="flex flex-col grow h-full w-full max-h-full overflow-hidden">
					<PageInterface
						appId={appId}
						event={routeEvent}
						config={parseUint8ArrayToJson(routeEvent.config) ?? {}}
						page={pageData}
						toolbarRef={headerRef}
						sidebarRef={sidebarRef}
					/>
				</div>
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
		if (!activeEvent) return <LoadingScreen />;
		if (!usableEvents) return <LoadingScreen />;

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
		routeEvent,
		sortedEvents,
		activeEvent,
		config,
		eventId,
		usableEvents,
	]);

	if (!appId) {
		return <NotFound />;
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
