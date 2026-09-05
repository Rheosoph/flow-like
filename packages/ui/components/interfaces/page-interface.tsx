"use client";

import { useTranslation } from "@flow-like/locales";
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
import { useAssetSource } from "../../hooks/use-asset-source";
import {
	type PageSurfaceIdentity,
	pageSurfaceCacheKey,
	pageSurfaceQueryKey,
	pageSurfaceRevision,
	pageSurfaceRouteKey,
	readPageSurfaceCache,
	writePageSurfaceCache,
} from "../../lib/page-surface-cache";
import { resolveEventBoardVersion } from "../../lib/schema/flow/board-version";
import type { PageSpecialEvent } from "../../lib/schema/flow/page-trigger";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
// By module path, not through the a2ui barrel: the barrel re-exports every component in the
// registry, which would pull the 3D scene and the mapping stack into every page load.
import { A2UIRenderer } from "../a2ui/A2UIRenderer";
import { DataProvider } from "../a2ui/DataContext";
import { LivePageAgentBridge } from "../a2ui/LivePageAgentBridge";
import {
	RouteDialogProvider,
	useRouteDialog,
} from "../a2ui/RouteDialogProvider";
import { applyA2UIMessage } from "../a2ui/apply-a2ui-message";
import { collectRunElements } from "../a2ui/collect-run-elements";
import type { ElementSource } from "../a2ui/element-materializer";
import { handleElementsRequestMessage } from "../a2ui/elements-request-handler";
import {
	type A2UINavigationMessageInterceptor,
	interceptA2UINavigationMessage,
} from "../a2ui/navigation-message";
import type {
	A2UIServerMessage,
	Surface,
	SurfaceComponent,
} from "../a2ui/types";
import { handleWidgetQueryMessage } from "../a2ui/widget-query-handler";
import { ScopedCustomCss } from "../scoped-custom-css";
import type { IUseInterfaceProps } from "./interfaces";
import { pageExecutionIdentity } from "./page-execution-identity";
import { PageLoadingSkeleton } from "./page-loading-skeleton";
import { shouldRevealProgressively } from "./progressive-page-reveal";

function isBackgroundClass(value: string | undefined): value is string {
	return value?.startsWith("bg-") ?? false;
}

export interface PageInterfaceProps extends IUseInterfaceProps {
	route?: string;
	/** Exact page payload returned by bootstrap for the served (primary or variant) target. */
	page: IPage;
	/** Exact page payload revision returned by a freshness-validating bootstrap read. */
	pageRevision?: string;
	/** Exact Page execution authority revision returned by bootstrap. */
	pageExecutionRevision?: string;
	/** Page-owned query state. Embedded pages pass this instead of inheriting the chat URL. */
	queryParams?: Record<string, string>;
	/** Consume page-owned route and query changes inside an embedded runtime. */
	onNavigationMessage?: A2UINavigationMessageInterceptor;
	/** False while an embedded runtime keeps this page mounted off screen. */
	active?: boolean;
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

	const rootComponentId = componentsRecord.root
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
	page,
	pageRevision,
	pageExecutionRevision,
	queryParams: providedQueryParams,
	onNavigationMessage,
	active = true,
}: PageInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const router = useRouter();
	const hostSearch = useSearchParams().toString();
	const runtimeQueryParams = useMemo(() => {
		if (providedQueryParams) return { ...providedQueryParams };
		const result: Record<string, string> = {};
		new URLSearchParams(hostSearch).forEach((value, key) => {
			result[key] = value;
		});
		return result;
	}, [hostSearch, providedQueryParams]);
	const search = useMemo(() => {
		const params = new URLSearchParams();
		for (const [key, value] of Object.entries(runtimeQueryParams).toSorted(
			([left], [right]) => left.localeCompare(right),
		)) {
			params.set(key, value);
		}
		return params.toString();
	}, [runtimeQueryParams]);
	const runtimeQueryParamsRef = useRef(runtimeQueryParams);
	runtimeQueryParamsRef.current = runtimeQueryParams;
	const auth = useAuth();
	const currentUserKey = auth?.user?.profile?.sub ?? "anonymous";
	const { openDialog, closeDialog } = useRouteDialog();
	const pageContainerId = useId();
	const [isLoadEventRunning, setIsLoadEventRunning] = useState(false);
	const [isScreenRevealed, setIsScreenRevealed] = useState(false);
	const [loadEventPhase, setLoadEventPhase] = useState<
		"idle" | "preparing" | "running"
	>("idle");
	const [completedLoadEventKey, setCompletedLoadEventKey] = useState<
		string | null
	>(null);
	const loadEventExecutedRef = useRef<string | null>(null);
	const [cachedSurfaceResult, setCachedSurfaceResult] = useState<{
		readonly identityKey: string;
		readonly surface: Surface | null;
	} | null>(null);

	const pageRoute = route || (config?.route as string);
	const isGovernedPage = Boolean(event.default_page_id);
	const cacheEnabled = page.cache === true;

	// A cached surface may only be replayed for the same parameters and the same account that
	// produced it: the onLoad workflow receives both, and its output is built from them.
	const surfaceIdentity = useMemo((): PageSurfaceIdentity | null => {
		const revision = pageSurfaceRevision(
			pageRevision ?? page.updatedAt,
			pageExecutionRevision,
		);
		if (!appId || !page.id || !revision) return null;
		return {
			appId,
			pageId: page.id,
			pageUpdatedAt: revision,
			routeKey: pageSurfaceRouteKey(pageRoute),
			queryKey: pageSurfaceQueryKey(search),
			userKey: currentUserKey,
		};
	}, [
		appId,
		page.id,
		page.updatedAt,
		pageRevision,
		pageExecutionRevision,
		currentUserKey,
		pageRoute,
		search,
	]);
	const surfaceIdentityKey = surfaceIdentity
		? pageSurfaceCacheKey(surfaceIdentity)
		: null;
	const shouldReadCachedSurface = Boolean(
		cacheEnabled && surfaceIdentityKey && page.onLoadEventId,
	);
	const isCacheLoading = Boolean(
		shouldReadCachedSurface &&
			cachedSurfaceResult?.identityKey !== surfaceIdentityKey,
	);
	const cachedSurface =
		cacheEnabled && cachedSurfaceResult?.identityKey === surfaceIdentityKey
			? cachedSurfaceResult.surface
			: null;
	const pageExecutionBoardId = event.board_id || page.boardId;
	const pageExecutionTargetIdentity = pageExecutionIdentity(
		pageExecutionBoardId,
		isGovernedPage ? event.id : undefined,
	);
	const pageExecutionVersion = useMemo(
		() =>
			resolveEventBoardVersion(
				event.board_id,
				event.board_version,
				pageExecutionBoardId,
			),
		[event.board_id, event.board_version, pageExecutionBoardId],
	);
	const loadEventExecutionKey = useMemo(() => {
		if (!page.onLoadEventId || !pageExecutionTargetIdentity) return null;
		return `${surfaceIdentityKey ?? page.id}:${page.onLoadEventId}:${pageExecutionTargetIdentity}:${pageExecutionVersion?.join(".") ?? "latest"}:${pageExecutionRevision ?? "unresolved"}`;
	}, [
		page.id,
		page.onLoadEventId,
		pageExecutionTargetIdentity,
		pageExecutionVersion,
		pageExecutionRevision,
		surfaceIdentityKey,
	]);
	const loadEventExecutionKeyRef = useRef(loadEventExecutionKey);
	loadEventExecutionKeyRef.current = loadEventExecutionKey;

	// Only pages that explicitly opt in keep a last rendered surface. Pages without the opt-in
	// always render their fresh page payload while the onLoad workflow updates it.
	useEffect(() => {
		let cancelled = false;

		if (
			!cacheEnabled ||
			!surfaceIdentity ||
			!surfaceIdentityKey ||
			!page.onLoadEventId
		) {
			return;
		}

		void readPageSurfaceCache(surfaceIdentity).then((surface) => {
			if (cancelled) return;
			setCachedSurfaceResult({ identityKey: surfaceIdentityKey, surface });
		});

		return () => {
			cancelled = true;
		};
	}, [cacheEnabled, surfaceIdentity, surfaceIdentityKey, page.onLoadEventId]);

	const initialSurface = useMemo(() => {
		if (cachedSurface) return cachedSurface;
		return buildSurfaceFromPage(page, page.id);
	}, [page, cachedSurface]);

	const { surface, handleServerMessage } = useManagedSurface(
		initialSurface,
		appId,
	);

	// Use ref to access current surface without creating dependency cycles
	const surfaceRef = useRef(surface);
	surfaceRef.current = surface;

	const elementSource = useCallback((): ElementSource | null => {
		const currentSurface = surfaceRef.current;
		if (!currentSurface) return null;
		return {
			surfaceId: currentSurface.id,
			components: currentSurface.components,
			storedValues: {},
		};
	}, []);

	// For opted-in pages, write only once the run that produced the surface has finished, so a
	// half-built surface is never what the next visit replays.
	useEffect(() => {
		if (!surfaceIdentity || !surface || isLoadEventRunning) return;
		if (!cacheEnabled || !page.onLoadEventId) return;
		if (
			!loadEventExecutionKey ||
			completedLoadEventKey !== loadEventExecutionKey
		)
			return;
		void writePageSurfaceCache(surfaceIdentity, surface);
	}, [
		surfaceIdentity,
		cacheEnabled,
		page.onLoadEventId,
		surface,
		isLoadEventRunning,
		loadEventExecutionKey,
		completedLoadEventKey,
	]);

	// Comprehensive A2UI message handler for page events
	const handleA2UIMessage = useCallback(
		(message: A2UIServerMessage) => {
			console.log("[PageInterface] A2UI message", { type: message.type });

			if (handleWidgetQueryMessage(message)) {
				return;
			}

			if (handleElementsRequestMessage(message, elementSource)) {
				return;
			}

			if (interceptA2UINavigationMessage(message, onNavigationMessage)) {
				return;
			}

			// Reveal the current screen while the workflow continues running.
			if (message.type === "showScreen") {
				setIsScreenRevealed(true);
				return;
			}

			if (shouldRevealProgressively(message)) {
				setIsScreenRevealed(true);
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
				console.log("[PageInterface] openDialog message received", {
					hasTitle: Boolean(title),
					queryParamKeys: Object.keys(queryParams ?? {}),
					hasDialogId: Boolean(dialogId),
				});
				openDialog(route, title, queryParams, dialogId);
				return;
			}

			// Handle close dialog
			if (message.type === "closeDialog") {
				const { dialogId } = message as { dialogId?: string };
				console.log("[PageInterface] closeDialog message received", {
					hasDialogId: Boolean(dialogId),
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
		[
			appId,
			router,
			openDialog,
			closeDialog,
			handleServerMessage,
			onNavigationMessage,
			elementSource,
		],
	);

	const pageContainerRef = useRef<HTMLDivElement | null>(null);

	// Helper to execute a page lifecycle event
	const executePageEvent = useCallback(
		async (
			specialEvent: PageSpecialEvent,
			eventName: string,
			extraPayload?: Record<string, unknown>,
			onRunStarted?: (runId: string) => void,
			isCurrent?: () => boolean,
		) => {
			if (!pageExecutionRevision) {
				console.warn(
					`[PageInterface] Missing governed Page context for ${eventName} event`,
				);
				return;
			}

			try {
				const currentSurface = surfaceRef.current;
				const surfaceElements = currentSurface
					? await collectRunElements({
							backend,
							appId,
							// The Event endpoint resolves the configured board. Omitting it here
							// avoids requiring Page users to read that board.
							boardId: undefined,
							surfaceId: currentSurface.id,
							components: currentSurface.components,
							storedValues: {},
						})
					: {};
				if (isCurrent && !isCurrent()) return;

				const payload = {
					id: `page_${specialEvent}`,
					payload: {
						_elements: surfaceElements,
						_elements_mode: "demand",
						_route: pageRoute || "/",
						_query_params: { ...runtimeQueryParamsRef.current },
						_page_id: page.id,
						_event_type: eventName,
						...extraPayload,
					},
				};

				const execFn =
					executionService?.executeEvent ?? backend.eventState.executeEvent;
				await execFn(
					appId,
					event.id,
					payload,
					false,
					onRunStarted,
					(events) => {
						if (isCurrent && !isCurrent()) return;
						for (const evt of events) {
							if (evt.event_type === "a2ui") {
								handleA2UIMessage(evt.payload as A2UIServerMessage);
							}
						}
					},
					undefined,
					{
						kind: "special",
						specialEvent,
						manifestRevision: pageExecutionRevision,
					},
				);
			} catch {
				console.error(`[PageInterface] Failed to execute ${eventName} event`);
			}
		},
		[
			appId,
			page,
			event.id,
			pageExecutionRevision,
			pageRoute,
			backend,
			executionService,
			handleA2UIMessage,
		],
	);

	// Execute onLoad event if configured (from page settings)
	useEffect(() => {
		const executeOnLoadEvent = async () => {
			if (!page.onLoadEventId || !loadEventExecutionKey) {
				loadEventExecutedRef.current = null;
				setCompletedLoadEventKey(null);
				setLoadEventPhase("idle");
				setIsLoadEventRunning(false);
				return;
			}

			// Query, account, and page revision are part of the key because onLoad receives and
			// can render data for all three.
			const executionKey = loadEventExecutionKey;
			if (loadEventExecutedRef.current === executionKey) return;
			loadEventExecutedRef.current = executionKey;

			setCompletedLoadEventKey(null);
			setIsScreenRevealed(false);
			setLoadEventPhase("preparing");
			setIsLoadEventRunning(true);
			try {
				await executePageEvent(
					"load",
					"onLoad",
					undefined,
					() => {
						if (
							loadEventExecutionKeyRef.current === executionKey &&
							loadEventExecutedRef.current === executionKey
						)
							setLoadEventPhase("running");
					},
					() =>
						loadEventExecutionKeyRef.current === executionKey &&
						loadEventExecutedRef.current === executionKey,
				);
			} finally {
				// A superseded run must not mark the current page as hydrated or stop its loader.
				if (loadEventExecutedRef.current === executionKey) {
					setCompletedLoadEventKey(executionKey);
					setLoadEventPhase("idle");
					setIsLoadEventRunning(false);
				}
			}
		};

		executeOnLoadEvent();
	}, [page, loadEventExecutionKey, executePageEvent]);

	// Execute onUnload event when page unmounts or user navigates away
	useEffect(() => {
		if (!page.onUnloadEventId) return;

		const handleBeforeUnload = () => {
			// Fire and forget - can't await in beforeunload
			executePageEvent("unload", "onUnload");
		};

		window.addEventListener("beforeunload", handleBeforeUnload);

		return () => {
			window.removeEventListener("beforeunload", handleBeforeUnload);
			// Also fire on component unmount (navigation within SPA)
			executePageEvent("unload", "onUnload");
		};
	}, [page.onUnloadEventId, executePageEvent]);

	// Execute onInterval event at configured time intervals
	const lastIntervalTickRef = useRef(0);
	useEffect(() => {
		if (
			!page.onIntervalEventId ||
			!page.onIntervalSeconds ||
			page.onIntervalSeconds <= 0
		)
			return;
		// An embedded runtime parks its host instead of unmounting, so a page nobody is
		// looking at is still mounted and would otherwise keep spending a board run every
		// tick, forever, invisibly.
		if (!active) return;

		const intervalMs = page.onIntervalSeconds * 1000;
		const tick = () => {
			lastIntervalTickRef.current = Date.now();
			executePageEvent("interval", "onInterval", {
				_interval_seconds: page.onIntervalSeconds,
			});
		};

		// Coming back on screen after more than a full period should show current data
		// rather than whatever was on the page when it parked.
		const sinceLastTick = Date.now() - lastIntervalTickRef.current;
		if (lastIntervalTickRef.current > 0 && sinceLastTick >= intervalMs) tick();

		const intervalId = setInterval(tick, intervalMs);
		return () => clearInterval(intervalId);
	}, [
		page.onIntervalEventId,
		page.onIntervalSeconds,
		executePageEvent,
		active,
	]);

	// Strip canvasSettings from the surface for A2UIRenderer. This component
	// already handles CSS injection and canvas styling at the outer level.
	// Passing it again would cause double CSS scoping and inline-style conflicts.
	const surfaceForRenderer = useMemo(() => {
		if (!surface) return null;
		if (!surface.canvasSettings) return surface;
		return { ...surface, canvasSettings: undefined };
	}, [surface]);

	const activeSurface = surface;
	const activeSurfaceForRenderer = surfaceForRenderer;

	const runtimeCanvasSettings =
		activeSurface?.canvasSettings ?? page.canvasSettings;
	// The background is the one asset with no component of its own to resolve it.
	const { src: backgroundImage } = useAssetSource(
		appId,
		runtimeCanvasSettings?.backgroundImage,
	);

	// The IndexedDB read is short and its result decides between real content and a skeleton,
	// so it is worth waiting for rather than flashing a placeholder it would have replaced.
	const shouldHoldForCachedState = isCacheLoading;
	const canRenderFromCache = Boolean(cachedSurface);
	const shouldShowLoading =
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

	if (isGovernedPage && !pageExecutionRevision) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>
					This Page could not load its execution authorization. Reload and try
					again.
				</p>
			</div>
		);
	}

	if (!activeSurface || !activeSurfaceForRenderer) {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>{t("noContentToDisplay", "No content to display")}</p>
			</div>
		);
	}

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
		backgroundImage: backgroundImage ? `url(${backgroundImage})` : undefined,
		backgroundSize: backgroundImage ? "cover" : undefined,
		backgroundPosition: backgroundImage ? "center" : undefined,
	};

	const customCss = runtimeCanvasSettings?.customCss;

	return (
		<div className="h-full w-full overflow-auto bg-background">
			<ScopedCustomCss
				css={customCss}
				scopeSelector={`[data-page-id="${pageContainerId}"]`}
			/>
			<div
				ref={pageContainerRef}
				data-page-id={pageContainerId}
				data-flowpilot-page-event-id={event.id}
				data-flowpilot-page-loading={isLoadEventRunning ? "true" : "false"}
				className={cn("min-h-full flex flex-col", backgroundClass)}
				style={canvasStyle}
			>
				<DataProvider initialData={[]}>
					<A2UIRenderer
						surface={activeSurfaceForRenderer}
						widgetRefs={page.widgetRefs}
						className="w-full flex-1"
						appId={appId}
						boardId={pageExecutionBoardId}
						boardVersion={pageExecutionVersion}
						eventId={event.id}
						governedPage={isGovernedPage}
						onA2UIMessage={handleA2UIMessage}
						onNavigationMessage={onNavigationMessage}
						isPreviewMode={true}
						openDialog={openDialog}
						closeDialog={closeDialog}
						agentBridge={
							appId ? (
								<LivePageAgentBridge
									appId={appId}
									pageId={activeSurface.id}
									eventId={event.id}
									getSurface={() => surfaceRef.current}
									getContainer={() => pageContainerRef.current}
									applyServerMessage={handleA2UIMessage}
									loading={isLoadEventRunning}
								/>
							) : undefined
						}
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
