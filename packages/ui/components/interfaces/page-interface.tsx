"use client";

import { useTranslation } from "@flow-like/locales";
import { useSearchParams } from "next/navigation";
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
	appQueryContext,
	readAppQuery,
	setAppQueryParam,
} from "../../lib/app-route-url";
import { useClientRouter } from "../../lib/client-navigation";
import { nativeWidgetPageQuery } from "../../lib/native-widget-page";
import {
	type PageSurfaceIdentity,
	pageSurfaceCacheKey,
	pageSurfaceQueryKey,
	pageSurfaceRevision,
	pageSurfaceRouteKey,
	readPageSurfaceCache,
	writePageSurfaceCache,
} from "../../lib/page-surface-cache";
import {
	type IRunTrace,
	startRunTrace,
	timeRunStep,
} from "../../lib/run-timing";
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
import {
	type RunElementDemand,
	collectRunElements,
} from "../a2ui/collect-run-elements";
import type { ElementSource } from "../a2ui/element-materializer";
import { handleElementsRequestMessage } from "../a2ui/elements-request-handler";
import { getFrontendStateStore } from "../a2ui/frontend-state";
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
import { NativeWidgetPageCaptureBridge } from "./native-widget-page-capture";
import { pageExecutionIdentity } from "./page-execution-identity";
import { PageLoadIndicator, PageLoadStatus } from "./page-load-indicator";
import {
	type ILoadRun,
	adoptLoadRunId,
	cancelLoadRun,
	createLoadRun,
} from "./page-load-run";
import { PageLoadingSkeleton } from "./page-loading-skeleton";
import { revealsPageLoad } from "./progressive-page-reveal";

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
	/** Page elements the Event's board reads, returned by bootstrap. */
	pageElementDemand?: RunElementDemand;
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
	pageElementDemand,
	queryParams: providedQueryParams,
	onNavigationMessage,
	active = true,
}: PageInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const frontendStateStore = getFrontendStateStore(appId);
	const router = useClientRouter();
	const hostSearch = useSearchParams().toString();
	const runtimeQueryContext = useMemo(() => {
		if (providedQueryParams)
			return { _query_params: { ...providedQueryParams } };
		return appQueryContext(hostSearch);
	}, [hostSearch, providedQueryParams]);
	const search = useMemo(() => {
		const params = providedQueryParams
			? new URLSearchParams(providedQueryParams)
			: readAppQuery(hostSearch);
		params.sort();
		return params.toString();
	}, [hostSearch, providedQueryParams]);
	const runtimeQueryContextRef = useRef(runtimeQueryContext);
	runtimeQueryContextRef.current = runtimeQueryContext;
	// Read at run time: a refetched bootstrap must not re-create the lifecycle callbacks.
	const pageElementDemandRef = useRef(pageElementDemand);
	pageElementDemandRef.current = pageElementDemand;
	const auth = useAuth();
	const currentUserKey = auth?.user?.profile?.sub ?? "anonymous";
	const { openDialog, closeDialog } = useRouteDialog();
	const pageContainerId = useId();
	const [isLoadEventRunning, setIsLoadEventRunning] = useState(false);
	const [revealedLoadEventKey, setRevealedLoadEventKey] = useState<
		string | null
	>(null);
	const [completedLoadEventKey, setCompletedLoadEventKey] = useState<
		string | null
	>(null);
	const [successfulLoadEventKey, setSuccessfulLoadEventKey] = useState<
		string | null
	>(null);
	const loadEventExecutedRef = useRef<string | null>(null);
	const loadRunRef = useRef<ILoadRun | null>(null);
	const loadRunTraceRef = useRef<IRunTrace | null>(null);
	const isMountedRef = useRef(false);
	const isDisposedRef = useRef(false);
	const markLoadRevealed = useCallback(() => {
		const trace = loadRunTraceRef.current;
		if (!trace) return;
		loadRunTraceRef.current = null;
		trace.mark("revealed");
		trace.finish();
	}, []);
	const releaseLoadRun = useCallback((reason: "superseded" | "unmounted") => {
		const run = loadRunRef.current;
		loadRunRef.current = null;
		if (run) cancelLoadRun(run);
		const trace = loadRunTraceRef.current;
		loadRunTraceRef.current = null;
		trace?.mark(reason);
		trace?.finish();
	}, []);
	const [cachedSurfaceResult, setCachedSurfaceResult] = useState<{
		readonly identityKey: string;
		readonly surface: Surface | null;
	} | null>(null);

	const pageRoute = route || (config?.route as string);
	const isGovernedPage = Boolean(event.default_page_id);
	const cacheEnabled = !page.noCache;

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
	const isScreenRevealed = Boolean(
		loadEventExecutionKey && revealedLoadEventKey === loadEventExecutionKey,
	);
	// A noCache page shows no layout that its onLoad run would replace.
	const isAwaitingFreshOutput = Boolean(
		page.noCache &&
			loadEventExecutionKey &&
			!isScreenRevealed &&
			completedLoadEventKey !== loadEventExecutionKey,
	);
	// Set as the load run reveals or ends, not at render, so a read landing in between still sees it.
	const freshLoadOutputKeyRef = useRef<string | null>(null);

	// The static layout renders at once; the last rendered surface replaces it when the read lands,
	// unless the load run has already put fresh output on screen.
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
			const superseded =
				freshLoadOutputKeyRef.current !== null &&
				freshLoadOutputKeyRef.current === loadEventExecutionKeyRef.current;
			setCachedSurfaceResult({
				identityKey: surfaceIdentityKey,
				surface: superseded ? null : surface,
			});
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

	// Write only once the run that produced the surface has succeeded, so a half-built or failed
	// surface is never what the next visit replays.
	useEffect(() => {
		if (!surfaceIdentity || !surface || isLoadEventRunning) return;
		if (!cacheEnabled || !page.onLoadEventId) return;
		if (
			!loadEventExecutionKey ||
			successfulLoadEventKey !== loadEventExecutionKey
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
		successfulLoadEventKey,
	]);

	// Comprehensive A2UI message handler for page events
	const handleA2UIMessage = useCallback(
		(message: A2UIServerMessage) => {
			console.log("[PageInterface] A2UI message", { type: message.type });
			if (frontendStateStore.handleMessage(message)) return;

			if (handleWidgetQueryMessage(message)) {
				return;
			}

			if (handleElementsRequestMessage(message, elementSource)) {
				return;
			}

			if (interceptA2UINavigationMessage(message, onNavigationMessage)) {
				return;
			}

			if (message.type === "showScreen") return;

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
				setAppQueryParam(url, key, value);

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
			frontendStateStore,
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
				return false;
			}
			if (specialEvent !== "load")
				loadRunTraceRef.current?.mark(`${specialEvent}_dispatch`);

			try {
				await timeRunStep("page.ensure_state", () =>
					frontendStateStore.ensureLoaded(page.id),
				);
				if (isCurrent && !isCurrent()) return false;
				const currentSurface = surfaceRef.current;
				const surfaceElements = currentSurface
					? await timeRunStep("page.collect_elements", () =>
							collectRunElements({
								backend,
								appId,
								// The Event endpoint resolves the configured board. Omitting it here
								// avoids requiring Page users to read that board.
								boardId: undefined,
								demand: pageElementDemandRef.current,
								surfaceId: currentSurface.id,
								components: currentSurface.components,
								storedValues: {},
							}),
						)
					: {};
				if (isCurrent && !isCurrent()) return false;
				const frontendState = frontendStateStore.getSnapshot();

				const payload = {
					id: `page_${specialEvent}`,
					payload: {
						_elements: surfaceElements,
						_elements_mode: "demand",
						_route: pageRoute || "/",
						...runtimeQueryContextRef.current,
						_page_id: page.id,
						_global_state: frontendState.globalState,
						_page_state: frontendState.pageStates[page.id] ?? {},
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
							if (evt.event_type !== "a2ui") continue;
							const message = evt.payload as A2UIServerMessage;
							if (specialEvent === "load" && revealsPageLoad(message)) {
								freshLoadOutputKeyRef.current =
									loadEventExecutionKeyRef.current;
								setRevealedLoadEventKey(loadEventExecutionKeyRef.current);
								markLoadRevealed();
							}
							handleA2UIMessage(message);
						}
					},
					undefined,
					{
						kind: "special",
						specialEvent,
						manifestRevision: pageExecutionRevision,
					},
				);
				return true;
			} catch {
				// A cancelled stream ends in an error once its page no longer owns the run.
				if (isCurrent && !isCurrent()) return false;
				console.error(`[PageInterface] Failed to execute ${eventName} event`);
				return false;
			}
		},
		[
			appId,
			page,
			frontendStateStore,
			event.id,
			pageExecutionRevision,
			pageRoute,
			backend,
			executionService,
			handleA2UIMessage,
			markLoadRevealed,
		],
	);

	// Execute onLoad event if configured (from page settings)
	useEffect(() => {
		const executeOnLoadEvent = async () => {
			if (!page.onLoadEventId || !loadEventExecutionKey) {
				releaseLoadRun("superseded");
				loadEventExecutedRef.current = null;
				setCompletedLoadEventKey(null);
				setSuccessfulLoadEventKey(null);
				setIsLoadEventRunning(false);
				return;
			}

			// Query, account, and page revision are part of the key because onLoad receives and
			// can render data for all three.
			const executionKey = loadEventExecutionKey;
			if (loadEventExecutedRef.current === executionKey) return;
			releaseLoadRun("superseded");
			loadEventExecutedRef.current = executionKey;
			const run = createLoadRun(backend.eventState);
			loadRunRef.current = run;
			const trace = startRunTrace("page onLoad");
			loadRunTraceRef.current = trace;
			const isCurrentRun = () =>
				!run.abandoned &&
				!isDisposedRef.current &&
				loadEventExecutionKeyRef.current === executionKey &&
				loadEventExecutedRef.current === executionKey;

			freshLoadOutputKeyRef.current = null;
			setCompletedLoadEventKey(null);
			setSuccessfulLoadEventKey(null);
			setRevealedLoadEventKey(null);
			setIsLoadEventRunning(true);
			let succeeded = false;
			try {
				succeeded = await executePageEvent(
					"load",
					"onLoad",
					undefined,
					(runId) => {
						if (adoptLoadRunId(run, runId)) trace.mark("run_initiated", runId);
					},
					isCurrentRun,
				);
			} finally {
				if (loadRunRef.current === run) loadRunRef.current = null;
				// A superseded run must not mark the current page as hydrated or stop its loader.
				if (!run.abandoned && loadEventExecutedRef.current === executionKey) {
					freshLoadOutputKeyRef.current = executionKey;
					setCompletedLoadEventKey(executionKey);
					setSuccessfulLoadEventKey(succeeded ? executionKey : null);
					setIsLoadEventRunning(false);
				}
				if (loadRunTraceRef.current === trace) loadRunTraceRef.current = null;
				trace.finish();
			}
		};

		executeOnLoadEvent();
	}, [page, loadEventExecutionKey, executePageEvent, releaseLoadRun, backend]);

	// StrictMode and Fast Refresh replay mount effects; only a page still unmounted after that
	// replay is really gone, and only then is its load run orphaned.
	useEffect(() => {
		isMountedRef.current = true;
		isDisposedRef.current = false;
		return () => {
			isMountedRef.current = false;
			setTimeout(() => {
				if (isMountedRef.current) return;
				isDisposedRef.current = true;
				if (loadRunRef.current) loadEventExecutedRef.current = null;
				releaseLoadRun("unmounted");
			}, 0);
		};
	}, [releaseLoadRun]);

	// onUnload belongs to a page that is leaving: a confirmed unmount, a switch to another page or
	// the window closing. A re-render that only renews executePageEvent is none of those.
	const unloadIdentity = `${event.id}:${page.id}`;
	const unloadIdentityRef = useRef(unloadIdentity);
	unloadIdentityRef.current = unloadIdentity;
	// Updated after commit, so a cleanup still sees the dispatch of the page that is leaving.
	const dispatchUnloadRef = useRef<(() => void) | null>(null);
	useEffect(() => {
		dispatchUnloadRef.current = page.onUnloadEventId
			? () => void executePageEvent("unload", "onUnload")
			: null;
	}, [page.onUnloadEventId, executePageEvent]);
	useEffect(() => {
		const handleBeforeUnload = () => dispatchUnloadRef.current?.();
		window.addEventListener("beforeunload", handleBeforeUnload);
		return () => {
			window.removeEventListener("beforeunload", handleBeforeUnload);
			const dispatchUnload = dispatchUnloadRef.current;
			setTimeout(() => {
				if (
					isMountedRef.current &&
					unloadIdentityRef.current === unloadIdentity
				)
					return;
				dispatchUnload?.();
			}, 0);
		};
	}, [unloadIdentity]);

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
			executePageEvent(
				"interval",
				"onInterval",
				{ _interval_seconds: page.onIntervalSeconds },
				undefined,
				() => !isDisposedRef.current,
			);
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

	if (isAwaitingFreshOutput) {
		return (
			<PageLoadingSkeleton title={t("runningWorkflow", "Running workflow")} />
		);
	}

	if (!activeSurface || !activeSurfaceForRenderer) {
		return (
			<div className="h-full w-full">
				<PageLoadStatus loading={isLoadEventRunning} />
				{isLoadEventRunning ? (
					<div aria-busy="true" className="h-full w-full bg-background">
						<PageLoadIndicator />
					</div>
				) : (
					<div className="flex items-center justify-center h-full text-muted-foreground">
						<p>{t("noContentToDisplay", "No content to display")}</p>
					</div>
				)}
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
	// The static layout renders at once; the load run fills it in behind a non-blocking bar.
	const isAwaitingLoadOutput = isLoadEventRunning && !isScreenRevealed;

	return (
		<div className="h-full w-full overflow-auto bg-background">
			<ScopedCustomCss
				css={customCss}
				scopeSelector={`[data-page-id="${pageContainerId}"]`}
			/>
			<PageLoadStatus loading={isAwaitingLoadOutput} />
			{isAwaitingLoadOutput && <PageLoadIndicator />}
			<div
				ref={pageContainerRef}
				aria-busy={isAwaitingLoadOutput || undefined}
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
						elementDemand={pageElementDemand}
						onA2UIMessage={handleA2UIMessage}
						onNavigationMessage={onNavigationMessage}
						isPreviewMode={true}
						openDialog={openDialog}
						closeDialog={closeDialog}
						agentBridge={
							appId ? (
								<>
									<NativeWidgetPageCaptureBridge
										key={JSON.stringify([
											appId,
											page.id,
											pageRevision ?? page.updatedAt,
											pageExecutionRevision,
										])}
										appId={appId}
										page={page}
										surface={activeSurface}
										path={pageRoute}
										search={
											providedQueryParams
												? search
												: nativeWidgetPageQuery(hostSearch)
										}
										pageRevision={
											pageSurfaceRevision(
												pageRevision ?? page.updatedAt,
												pageExecutionRevision,
											) ?? page.updatedAt
										}
										ready={
											active &&
											!auth?.isLoading &&
											!isLoadEventRunning &&
											(!page.onLoadEventId ||
												Boolean(
													loadEventExecutionKey &&
														successfulLoadEventKey === loadEventExecutionKey,
												))
										}
									/>
									<LivePageAgentBridge
										appId={appId}
										pageId={activeSurface.id}
										eventId={event.id}
										getSurface={() => surfaceRef.current}
										getContainer={() => pageContainerRef.current}
										applyServerMessage={handleA2UIMessage}
										loading={isLoadEventRunning}
									/>
								</>
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
