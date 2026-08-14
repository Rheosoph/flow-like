"use client";

import { useTranslation } from "@flow-like/locales";
import { Settings } from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { boardReadinessKey, whenBoardReady } from "../../lib/board-readiness";
import {
	type PageSurfaceIdentity,
	pageSurfaceQueryKey,
	readPageSurfaceCache,
	writePageSurfaceCache,
} from "../../lib/page-surface-cache";
import { presignPageContent } from "../../lib/presign-assets";
import {
	resolveEventBoardVersion,
	withBoardVersion,
} from "../../lib/schema/flow/board-version";
import type { IEvent } from "../../lib/schema/flow/event";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import type { IStorageState } from "../../state/backend-state/storage-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
// By module path, not through the a2ui barrel: the barrel re-exports every component in the
// registry, which would pull the 3D scene and the mapping stack into every page load.
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { DataProvider } from "../a2ui/DataContext";
import {
	RouteDialogProvider,
	useRouteDialog,
} from "../a2ui/RouteDialogProvider";
import { applyA2UIMessage } from "../a2ui/apply-a2ui-message";
import type {
	A2UIServerMessage,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import { handleWidgetQueryMessage } from "../a2ui/widget-query-handler";
import { ScopedCustomCss } from "../scoped-custom-css";
import type { IUseInterfaceProps } from "./interfaces";
import { PageLoadingSkeleton } from "./page-loading-skeleton";

function isBackgroundClass(value: string | undefined): value is string {
	return value?.startsWith("bg-") ?? false;
}

interface WidgetWarmup {
	readonly promise: Promise<void>;
	completedAt?: number;
}

/**
 * Warming widget definitions is shared across every page of an app, so the record outlives any
 * one interface. It expires so that widgets published mid-session still reach local execution.
 */
const widgetWarmups = new Map<string, WidgetWarmup>();
const WIDGET_WARMUP_TTL_MS = 60_000;

export interface PageInterfaceProps extends Omit<IUseInterfaceProps, "event"> {
	event?: IUseInterfaceProps["event"];
	route?: string;
	page?: IPage;
}

/**
 * Signs every storage-backed asset the page renders in one request.
 *
 * A failure here is cosmetic — the unsigned paths simply do not resolve — so it must never
 * stop the page from rendering the rest of its content.
 */
async function presignPage(
	appId: string,
	page: IPage,
	storageState: IStorageState,
): Promise<IPage> {
	try {
		const { components, backgroundImage } = await presignPageContent(
			appId,
			page.components ?? [],
			page.canvasSettings,
			storageState,
		);

		return {
			...page,
			components,
			canvasSettings: page.canvasSettings
				? { ...page.canvasSettings, backgroundImage }
				: page.canvasSettings,
		};
	} catch (presignError) {
		console.warn(
			"[PageInterface] Failed to presign page assets:",
			presignError,
		);
		return page;
	}
}

function buildSurfaceFromPage(page: IPage, pageId: string): Surface | null {
	if (!page.components || page.components.length === 0) {
		return null;
	}

	const componentsRecord = page.components.reduce(
		(acc, comp) => {
			acc[comp.id] = comp;
			return acc;
		},
		{} as Record<string, SurfaceComponent>,
	);

	const rootComponentId = componentsRecord["root"]
		? "root"
		: page.components[0]?.id || "";

	return {
		id: pageId,
		rootComponentId,
		components: componentsRecord,
		canvasSettings: page.canvasSettings,
	};
}

function useManagedSurface(initialSurface: Surface | null, appId?: string) {
	const [surface, setSurface] = useState<Surface | null>(initialSurface);
	const prevInitialSurfaceRef = useRef<Surface | null>(initialSurface);

	// Sync initialSurface → surface during render (no one-render lag)
	if (initialSurface !== prevInitialSurfaceRef.current) {
		prevInitialSurfaceRef.current = initialSurface;
		setSurface(initialSurface);
	}

	const handleServerMessage = useCallback((message: A2UIServerMessage) => {
		setSurface((prevSurface) =>
			prevSurface ? applyA2UIMessage(prevSurface, message) : prevSurface,
		);
	}, []);

	return { surface, handleServerMessage };
}

function PageInterfaceInner({
	appId,
	event,
	config,
	route,
	page: providedPage,
}: PageInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const router = useRouter();
	const search = useSearchParams().toString();
	const auth = useAuth();
	const currentUserKey = auth?.user?.profile?.sub ?? "anonymous";
	const { openDialog, closeDialog } = useRouteDialog();
	const pageContainerId = useId();
	const [page, setPage] = useState<IPage | null>(null);
	const [routeMapping, setRouteMapping] = useState<IRouteMapping | null>(null);
	const [routeEvent, setRouteEvent] = useState<IEvent | null>(null);
	const [isLoading, setIsLoading] = useState(true);
	const [isLoadEventRunning, setIsLoadEventRunning] = useState(false);
	const [isCacheLoading, setIsCacheLoading] = useState(false);
	const [isScreenRevealed, setIsScreenRevealed] = useState(false);
	const [loadEventPhase, setLoadEventPhase] = useState<
		"idle" | "preparing" | "running"
	>("idle");
	const [error, setError] = useState<string | null>(null);
	const loadEventExecutedRef = useRef<string | null>(null);
	const [cachedSurface, setCachedSurface] = useState<Surface | null>(null);

	const pageRoute = route || (config?.route as string);
	const cacheSource = providedPage ?? page;
	const activePageEvent = event ?? routeEvent;

	// A cached surface may only be replayed for the same parameters and the same account that
	// produced it: the onLoad workflow receives both, and its output is built from them.
	const surfaceIdentity = useMemo((): PageSurfaceIdentity | null => {
		if (!appId || !cacheSource?.id || !cacheSource.updatedAt) return null;
		return {
			appId,
			pageId: cacheSource.id,
			pageUpdatedAt: cacheSource.updatedAt,
			queryKey: pageSurfaceQueryKey(search),
			userKey: currentUserKey,
		};
	}, [appId, cacheSource?.id, cacheSource?.updatedAt, currentUserKey, search]);
	const pageExecutionBoardId = activePageEvent?.board_id || page?.boardId;
	const pageExecutionVersion = useMemo(
		() =>
			resolveEventBoardVersion(
				activePageEvent?.board_id,
				activePageEvent?.board_version,
				pageExecutionBoardId,
			),
		[
			activePageEvent?.board_id,
			activePageEvent?.board_version,
			pageExecutionBoardId,
		],
	);

	useEffect(() => {
		if (providedPage) {
			const presignProvidedPage = async () => {
				setPage(await presignPage(appId, providedPage, backend.storageState));
				setIsLoading(false);
			};
			presignProvidedPage();
			return;
		}

		const loadPageFromRoute = async () => {
			setIsLoading(true);
			setError(null);
			try {
				// Get route mapping (path -> eventId)
				let mapping: IRouteMapping | null = null;

				if (pageRoute) {
					mapping = await backend.routeState.getRouteByPath(appId, pageRoute);
				} else {
					mapping = await backend.routeState.getDefaultRoute(appId);
				}

				if (!mapping) {
					setRouteMapping(null);
					setRouteEvent(null);
					setPage(null);
					setIsLoading(false);
					return;
				}

				setRouteMapping(mapping);

				// Get the event to determine what to display
				const eventData = await backend.eventState.getEvent(
					appId,
					mapping.eventId,
				);
				setRouteEvent(eventData);

				// If event has default_page_id, it's a page-target event
				if (eventData.default_page_id) {
					const pageResult = await backend.pageState.getPage(
						appId,
						eventData.default_page_id,
						eventData.board_id || undefined,
					);
					if (pageResult) {
						setPage(await presignPage(appId, pageResult, backend.storageState));
					} else {
						setError(`Page not found: ${eventData.default_page_id}`);
					}
				} else {
					// Board-target event - no page to display
					setPage(null);
				}
			} catch (e) {
				console.error("Failed to load page:", e);
				setError("Failed to load page");
			} finally {
				setIsLoading(false);
			}
		};

		loadPageFromRoute();
	}, [
		appId,
		pageRoute,
		providedPage,
		backend.routeState,
		backend.pageState,
		backend.eventState,
		backend.storageState,
	]);

	// Every page keeps its last rendered surface, not only those that opt in: the alternative
	// to showing it is a skeleton for the whole length of the onLoad run, and the run replaces
	// it either way. Pages without an onLoad event build their surface from the payload and
	// never consult this.
	useEffect(() => {
		let cancelled = false;

		if (!surfaceIdentity || !cacheSource?.onLoadEventId) {
			setCachedSurface(null);
			setIsCacheLoading(false);
			return;
		}

		setIsCacheLoading(true);
		void readPageSurfaceCache(surfaceIdentity)
			.then((surface) => {
				if (cancelled) return;
				setCachedSurface(surface);
			})
			.finally(() => {
				if (cancelled) return;
				setIsCacheLoading(false);
			});

		return () => {
			cancelled = true;
		};
	}, [surfaceIdentity, cacheSource?.onLoadEventId]);

	const initialSurface = useMemo(() => {
		if (cachedSurface) return cachedSurface;
		if (!page) return null;
		return buildSurfaceFromPage(page, page.id);
	}, [page, cachedSurface]);

	const { surface, handleServerMessage } = useManagedSurface(
		initialSurface,
		appId,
	);

	// Written only once the run that produced it has finished, so a half-built surface is never
	// what the next visit replays.
	useEffect(() => {
		if (!surfaceIdentity || !surface || isLoadEventRunning) return;
		if (!page?.onLoadEventId) return;
		void writePageSurfaceCache(surfaceIdentity, surface);
	}, [surfaceIdentity, page?.onLoadEventId, surface, isLoadEventRunning]);

	// Comprehensive A2UI message handler for page events
	const handleA2UIMessage = useCallback(
		(message: A2UIServerMessage) => {
			console.log("[PageInterface] A2UI message:", message.type, message);

			if (handleWidgetQueryMessage(message)) {
				return;
			}

			// Reveal the current screen while the workflow continues running.
			if (message.type === "showScreen") {
				setIsScreenRevealed(true);
				return;
			}

			// Handle navigation
			if (message.type === "navigateTo") {
				const { route, replace, queryParams } = message as {
					route: string;
					replace: boolean;
					queryParams?: Record<string, string>;
				};

				let navUrl = route;
				if (appId && !route.startsWith("/use") && !route.startsWith("http")) {
					// Parse any query params that might be in the route itself
					const [routePath, routeQueryString] = route.split("?");
					const params = new URLSearchParams();
					params.set("id", appId);
					params.set("route", routePath);

					// Add query params from the route string
					if (routeQueryString) {
						const routeParams = new URLSearchParams(routeQueryString);
						routeParams.forEach((value, key) => {
							params.set(key, value);
						});
					}

					// Add additional query params (these override route params)
					if (queryParams) {
						for (const [key, value] of Object.entries(queryParams)) {
							params.set(key, value);
						}
					}
					navUrl = `/use?${params.toString()}`;
				} else if (queryParams && Object.keys(queryParams).length > 0) {
					const params = new URLSearchParams(queryParams);
					const separator = navUrl.includes("?") ? "&" : "?";
					navUrl = `${navUrl}${separator}${params.toString()}`;
				}

				if (replace) {
					router.replace(navUrl);
				} else {
					router.push(navUrl);
				}
				return;
			}

			// Handle open dialog
			if (message.type === "openDialog") {
				const { route, title, queryParams, dialogId } = message as {
					route: string;
					title?: string;
					queryParams?: Record<string, string>;
					dialogId?: string;
				};
				console.log("[PageInterface] openDialog message received:", {
					route,
					title,
					queryParams,
					dialogId,
				});
				openDialog(route, title, queryParams, dialogId);
				return;
			}

			// Handle close dialog
			if (message.type === "closeDialog") {
				const { dialogId } = message as { dialogId?: string };
				console.log("[PageInterface] closeDialog message received:", {
					dialogId,
				});
				closeDialog(dialogId);
				return;
			}

			// Handle query param updates
			if (message.type === "setQueryParam") {
				const { key, value, replace } = message as {
					key: string;
					value?: string;
					replace: boolean;
				};

				const url = new URL(window.location.href);
				if (value === undefined || value === "") {
					url.searchParams.delete(key);
				} else {
					url.searchParams.set(key, value);
				}

				if (replace) {
					router.replace(url.pathname + url.search);
				} else {
					router.push(url.pathname + url.search);
				}
				return;
			}

			// Handle element updates
			handleServerMessage(message);
		},
		[appId, router, openDialog, closeDialog, handleServerMessage],
	);

	// Use ref to access current surface without creating dependency cycles
	const surfaceRef = useRef(surface);
	surfaceRef.current = surface;

	// Build elements from surface components for the workflow payload
	// Uses ref to avoid dependency on surface changing
	const getElementsFromSurface = useCallback(() => {
		const currentSurface = surfaceRef.current;
		if (!currentSurface) return {};
		const elements: Record<string, unknown> = {};
		for (const [componentId, surfaceComponent] of Object.entries(
			currentSurface.components,
		)) {
			const elementId = `${currentSurface.id}/${componentId}`;
			elements[elementId] = {
				...surfaceComponent,
				__element_id: elementId,
			};
		}
		return elements;
	}, []); // No dependencies - uses ref

	const prepareLocalWidgetDefinitions = useCallback(async () => {
		if (!appId || !backend.capabilities().canExecuteLocally) return;

		const tracked = widgetWarmups.get(appId);
		// A completed warm-up was kept forever, so a widget published or edited later in the
		// session never reached local execution. Concurrent runs still share one pass; a
		// finished one only counts as current for as long as the list plausibly is.
		if (
			tracked &&
			(tracked.completedAt === undefined ||
				Date.now() - tracked.completedAt < WIDGET_WARMUP_TTL_MS)
		) {
			await tracked.promise.catch(() => undefined);
			return;
		}

		const entry: WidgetWarmup = {
			promise: (async () => {
				const widgets = await backend.widgetState.getWidgets(appId);
				const results = await Promise.allSettled(
					widgets.map(([widgetAppId, widgetId]) =>
						backend.widgetState.getWidget(widgetAppId, widgetId),
					),
				);
				const failedCount = results.filter(
					(result) => result.status === "rejected",
				).length;
				if (failedCount > 0) {
					console.warn(
						`[PageInterface] Failed to warm ${failedCount} widget definition(s) before local execution`,
					);
				}
			})(),
		};

		widgetWarmups.set(appId, entry);
		try {
			await entry.promise;
			entry.completedAt = Date.now();
		} catch (e) {
			// A failed pass must not be remembered as current, or the next run inherits it.
			if (widgetWarmups.get(appId) === entry) widgetWarmups.delete(appId);
			console.warn(
				"[PageInterface] Failed to warm widget definitions before local execution:",
				e,
			);
		}
	}, [appId, backend]);

	// Helper to execute a page lifecycle event
	const executePageEvent = useCallback(
		async (
			eventNodeId: string | undefined,
			eventName: string,
			extraPayload?: Record<string, unknown>,
			onRunStarted?: (runId: string) => void,
		) => {
			if (!eventNodeId || !page) return;

			const boardId = pageExecutionBoardId;
			if (!boardId) {
				console.warn(`[PageInterface] No boardId for ${eventName} event`);
				return;
			}

			try {
				// Get component data from surface (for GetElement to work)
				const surfaceElements = getElementsFromSurface();

				const queryParams: Record<string, string> = {};
				if (typeof window !== "undefined") {
					const searchParams = new URLSearchParams(window.location.search);
					searchParams.forEach((value, key) => {
						queryParams[key] = value;
					});
				}

				const payload = withBoardVersion(
					{
						id: eventNodeId,
						payload: {
							_elements: surfaceElements,
							_route: pageRoute || "/",
							_query_params: queryParams,
							_page_id: page.id,
							_event_type: eventName,
							...extraPayload,
						},
					},
					pageExecutionVersion,
				);

				// Use execution service if available (checks runtime variables)
				const execFn =
					executionService?.executeBoard ?? backend.boardState.executeBoard;
				// The render no longer waits for the board refresh, so the run does: a device
				// that has only ever synced the board manifest must not execute the stale copy.
				await Promise.all([
					whenBoardReady(
						boardReadinessKey(
							appId,
							boardId,
							pageExecutionVersion ?? undefined,
						),
					),
					prepareLocalWidgetDefinitions(),
				]);
				await execFn(appId, boardId, payload, false, onRunStarted, (events) => {
					for (const evt of events) {
						if (evt.event_type === "a2ui") {
							handleA2UIMessage(evt.payload as A2UIServerMessage);
						}
					}
				});
			} catch (e) {
				console.error(
					`[PageInterface] Failed to execute ${eventName} event:`,
					e,
				);
			}
		},
		[
			appId,
			page,
			pageExecutionBoardId,
			pageExecutionVersion,
			pageRoute,
			backend.boardState,
			executionService,
			handleA2UIMessage,
			getElementsFromSurface,
			prepareLocalWidgetDefinitions,
		],
	);

	// Execute onLoad event if configured (from page settings)
	useEffect(() => {
		const executeOnLoadEvent = async () => {
			if (!page?.onLoadEventId) return;

			const boardId = pageExecutionBoardId;
			if (!boardId) return;

			// Prevent duplicate execution for the same page + event combination
			const executionKey = `${page.id}:${page.onLoadEventId}:${boardId}:${pageExecutionVersion?.join(".") ?? "latest"}`;
			if (loadEventExecutedRef.current === executionKey) return;
			loadEventExecutedRef.current = executionKey;

			setIsScreenRevealed(false);
			setLoadEventPhase("preparing");
			setIsLoadEventRunning(true);
			try {
				await executePageEvent(page.onLoadEventId, "onLoad", undefined, () =>
					setLoadEventPhase("running"),
				);
			} finally {
				setLoadEventPhase("idle");
				setIsLoadEventRunning(false);
			}
		};

		executeOnLoadEvent();
	}, [
		appId,
		page,
		pageExecutionBoardId,
		pageExecutionVersion,
		executePageEvent,
	]);

	// Execute onUnload event when page unmounts or user navigates away
	useEffect(() => {
		if (!page?.onUnloadEventId) return;

		const handleBeforeUnload = () => {
			// Fire and forget - can't await in beforeunload
			executePageEvent(page.onUnloadEventId, "onUnload");
		};

		window.addEventListener("beforeunload", handleBeforeUnload);

		return () => {
			window.removeEventListener("beforeunload", handleBeforeUnload);
			// Also fire on component unmount (navigation within SPA)
			executePageEvent(page.onUnloadEventId, "onUnload");
		};
	}, [page?.onUnloadEventId, executePageEvent]);

	// Execute onInterval event at configured time intervals
	useEffect(() => {
		if (
			!page?.onIntervalEventId ||
			!page?.onIntervalSeconds ||
			page.onIntervalSeconds <= 0
		)
			return;

		const intervalMs = page.onIntervalSeconds * 1000;

		const intervalId = setInterval(() => {
			executePageEvent(page.onIntervalEventId, "onInterval", {
				_interval_seconds: page.onIntervalSeconds,
			});
		}, intervalMs);

		return () => clearInterval(intervalId);
	}, [page?.onIntervalEventId, page?.onIntervalSeconds, executePageEvent]);

	// Strip canvasSettings from the surface for A2UIRenderer — this component
	// already handles CSS injection and canvas styling at the outer level.
	// Passing it again would cause double CSS scoping and inline-style conflicts.
	const surfaceForRenderer = useMemo(() => {
		if (!surface) return null;
		if (!surface.canvasSettings) return surface;
		return { ...surface, canvasSettings: undefined };
	}, [surface]);

	const activeSurface = surface;
	const activeSurfaceForRenderer = surfaceForRenderer;

	// The IndexedDB read is short and its result decides between real content and a skeleton,
	// so it is worth waiting for rather than flashing a placeholder it would have replaced.
	const shouldHoldForCachedState = isCacheLoading;
	const canRenderFromCache = Boolean(cachedSurface);
	const shouldShowLoading =
		(isLoading && !canRenderFromCache) ||
		shouldHoldForCachedState ||
		(isLoadEventRunning && !canRenderFromCache && !isScreenRevealed);
	const loadingTitle = isLoadEventRunning
		? loadEventPhase === "running"
			? "Running workflow"
			: "Preparing workflow"
		: "Loading page";
	if (shouldShowLoading) {
		return <PageLoadingSkeleton title={loadingTitle} />;
	}

	if (error) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>{error}</p>
			</div>
		);
	}

	if (!routeMapping && !providedPage) {
		return (
			<div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground">
				<p>{t('noRouteConfiguredForThisPath', 'No route configured for this path')}</p>
				<Link
					href={`/library/config/pages?appId=${appId}`}
					className="flex items-center gap-2 text-sm hover:text-foreground transition-colors"
				>
					<Settings className="h-4 w-4" />
					{t('configureRoutes', 'Configure routes')}
				</Link>
			</div>
		);
	}

	if (routeEvent && !routeEvent.default_page_id) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>{t('eventDoesNotHaveAPageTarget', 'Event does not have a page target')}</p>
			</div>
		);
	}

	if (!activeSurface) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>{t('noContentToDisplay', 'No content to display')}</p>
			</div>
		);
	}

	const runtimeCanvasSettings =
		activeSurface?.canvasSettings ?? page?.canvasSettings;
	const backgroundClass = isBackgroundClass(
		runtimeCanvasSettings?.backgroundColor,
	)
		? runtimeCanvasSettings?.backgroundColor
		: undefined;

	const canvasStyle: React.CSSProperties = {
		backgroundColor: backgroundClass
			? undefined
			: runtimeCanvasSettings?.backgroundColor,
		padding: runtimeCanvasSettings?.padding,
		backgroundImage: runtimeCanvasSettings?.backgroundImage
			? `url(${runtimeCanvasSettings.backgroundImage})`
			: undefined,
		backgroundSize: runtimeCanvasSettings?.backgroundImage
			? "cover"
			: undefined,
		backgroundPosition: runtimeCanvasSettings?.backgroundImage
			? "center"
			: undefined,
	};

	const customCss = runtimeCanvasSettings?.customCss;

	return (
		<div className="h-full w-full overflow-auto bg-background">
			<ScopedCustomCss
				css={customCss}
				scopeSelector={`[data-page-id="${pageContainerId}"]`}
			/>
			<div
				data-page-id={pageContainerId}
				data-flowpilot-page-event-id={activePageEvent?.id ?? ""}
				data-flowpilot-page-loading={isLoadEventRunning ? "true" : "false"}
				className={cn("min-h-full flex flex-col", backgroundClass)}
				style={canvasStyle}
			>
				<DataProvider initialData={[]}>
					<A2UIRenderer
						surface={activeSurfaceForRenderer!}
						widgetRefs={page?.widgetRefs}
						className="w-full flex-1"
						appId={appId}
						boardId={pageExecutionBoardId}
						boardVersion={pageExecutionVersion}
						eventId={activePageEvent?.id}
						onA2UIMessage={handleA2UIMessage}
						isPreviewMode={true}
						openDialog={openDialog}
						closeDialog={closeDialog}
					/>
				</DataProvider>
			</div>
		</div>
	);
}

export function PageInterface(props: PageInterfaceProps) {
	return (
		<RouteDialogProvider appId={props.appId}>
			<PageInterfaceInner {...props} />
		</RouteDialogProvider>
	);
}

export function usePageInterface(props: PageInterfaceProps) {
	return <PageInterface {...props} />;
}
