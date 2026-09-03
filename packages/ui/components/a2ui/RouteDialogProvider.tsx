"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
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
import type { IEvent } from "../../lib/schema/flow/event";
import { useBackend } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import { PageLoadingSkeleton } from "../interfaces/page-loading-skeleton";
import { shouldRevealProgressively } from "../interfaces/progressive-page-reveal";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import { A2UIRenderer } from "./A2UIRenderer";
import { applyA2UIMessage } from "./apply-a2ui-message";
import { collectRunElements } from "./collect-run-elements";
import type { ElementSource } from "./element-materializer";
import { handleElementsRequestMessage } from "./elements-request-handler";
import type { A2UIServerMessage, Surface, SurfaceComponent } from "./types";
import { handleWidgetQueryMessage } from "./widget-query-handler";

interface DialogState {
	id: string;
	route: string;
	title?: string;
	queryParams?: Record<string, string>;
	isOpen: boolean;
}

interface RouteDialogContextValue {
	openDialog: (
		route: string,
		title?: string,
		queryParams?: Record<string, string>,
		dialogId?: string,
	) => void;
	closeDialog: (dialogId?: string) => void;
	dialogs: DialogState[];
}

const RouteDialogContext = createContext<RouteDialogContextValue | null>(null);

export function useRouteDialog() {
	const context = useContext(RouteDialogContext);
	if (!context) {
		throw new Error("useRouteDialog must be used within RouteDialogProvider");
	}
	return context;
}

export function useRouteDialogSafe() {
	return useContext(RouteDialogContext);
}

interface RouteDialogProviderProps {
	children: ReactNode;
	appId?: string;
}

export function RouteDialogProvider({
	children,
	appId,
}: RouteDialogProviderProps) {
	const [dialogs, setDialogs] = useState<DialogState[]>([]);

	const openDialog = useCallback(
		(
			route: string,
			title?: string,
			queryParams?: Record<string, string>,
			dialogId?: string,
		) => {
			console.log("[RouteDialogProvider] openDialog called:", {
				route,
				title,
				queryParams,
				dialogId,
			});
			const id = dialogId || `dialog-${Date.now()}`;
			setDialogs((prev) => {
				console.log("[RouteDialogProvider] Adding dialog to stack:", {
					id,
					route,
					prevCount: prev.length,
				});
				return [...prev, { id, route, title, queryParams, isOpen: true }];
			});
		},
		[],
	);

	const closeDialog = useCallback((dialogId?: string) => {
		setDialogs((prev) => {
			if (dialogId) {
				return prev.filter((d) => d.id !== dialogId);
			}
			// Close the topmost dialog
			if (prev.length === 0) return prev;
			return prev.slice(0, -1);
		});
	}, []);

	const handleDialogOpenChange = useCallback(
		(dialogId: string, open: boolean) => {
			if (!open) {
				setDialogs((prev) => prev.filter((d) => d.id !== dialogId));
			}
		},
		[],
	);

	return (
		<RouteDialogContext.Provider value={{ openDialog, closeDialog, dialogs }}>
			{children}
			{dialogs.map((dialog) => (
				<RouteDialogRenderer
					key={dialog.id}
					dialog={dialog}
					appId={appId}
					onOpenChange={(open) => handleDialogOpenChange(dialog.id, open)}
					openDialog={openDialog}
					closeDialog={closeDialog}
				/>
			))}
		</RouteDialogContext.Provider>
	);
}

interface RouteDialogRendererProps {
	dialog: DialogState;
	appId?: string;
	onOpenChange: (open: boolean) => void;
	openDialog: (
		route: string,
		title?: string,
		queryParams?: Record<string, string>,
		dialogId?: string,
	) => void;
	closeDialog: (dialogId?: string) => void;
}

function RouteDialogRenderer({
	dialog,
	appId,
	onOpenChange,
	openDialog,
	closeDialog,
}: RouteDialogRendererProps) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const auth = useAuth();
	const currentUserKey = auth?.user?.profile?.sub ?? "anonymous";
	const [isLoading, setIsLoading] = useState(true);
	const [isLoadEventRunning, setIsLoadEventRunning] = useState(false);
	const [isScreenRevealed, setIsScreenRevealed] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [surface, setSurface] = useState<Surface | null>(null);
	const [page, setPage] = useState<IPage | null>(null);
	const [routeEvent, setRouteEvent] = useState<IEvent | null>(null);
	const [pageRevision, setPageRevision] = useState<string | null>(null);
	const [pageExecutionRevision, setPageExecutionRevision] = useState<
		string | null
	>(null);
	const [completedLoadEventKey, setCompletedLoadEventKey] = useState<
		string | null
	>(null);
	const loadEventExecutedRef = useRef<string | null>(null);
	const [cachedSurfaceResult, setCachedSurfaceResult] = useState<{
		readonly identityKey: string;
		readonly surface: Surface | null;
	} | null>(null);
	const pageExecutionBoardId = routeEvent?.board_id || page?.boardId;
	const pageExecutionVersion = useMemo(
		() =>
			resolveEventBoardVersion(
				routeEvent?.board_id,
				routeEvent?.board_version,
				pageExecutionBoardId,
			),
		[routeEvent?.board_id, routeEvent?.board_version, pageExecutionBoardId],
	);
	const isGovernedPage = Boolean(routeEvent?.default_page_id);

	// A dialog is addressed by its own parameters, not the host page's, so those are what its
	// cached surface is keyed by.
	const dialogQueryKey = useMemo(
		() =>
			pageSurfaceQueryKey(
				new URLSearchParams(dialog.queryParams ?? {}).toString(),
			),
		[dialog.queryParams],
	);
	const surfaceIdentity = useMemo((): PageSurfaceIdentity | null => {
		const revision = pageSurfaceRevision(
			pageRevision ?? page?.updatedAt,
			pageExecutionRevision,
		);
		if (!appId || !page?.id || !revision) return null;
		return {
			appId,
			pageId: page.id,
			pageUpdatedAt: revision,
			routeKey: pageSurfaceRouteKey(dialog.route),
			queryKey: dialogQueryKey,
			userKey: currentUserKey,
		};
	}, [
		appId,
		page?.id,
		page?.updatedAt,
		pageRevision,
		pageExecutionRevision,
		dialog.route,
		dialogQueryKey,
		currentUserKey,
	]);
	const surfaceIdentityKey = surfaceIdentity
		? pageSurfaceCacheKey(surfaceIdentity)
		: null;
	const cacheEnabled = page?.cache === true;
	const shouldReadCachedSurface = Boolean(
		cacheEnabled && surfaceIdentityKey && page?.onLoadEventId,
	);
	const isCacheLoading = Boolean(
		shouldReadCachedSurface &&
			cachedSurfaceResult?.identityKey !== surfaceIdentityKey,
	);
	const cachedSurface =
		cacheEnabled && cachedSurfaceResult?.identityKey === surfaceIdentityKey
			? cachedSurfaceResult.surface
			: null;
	const loadEventExecutionKey = useMemo(() => {
		if (!page?.onLoadEventId || !pageExecutionBoardId) return null;
		return `${dialog.id}:${surfaceIdentityKey ?? page.id}:${page.onLoadEventId}:${pageExecutionBoardId}:${pageExecutionVersion?.join(".") ?? "latest"}:${pageExecutionRevision ?? "unresolved"}`;
	}, [
		dialog.id,
		page?.id,
		page?.onLoadEventId,
		pageExecutionBoardId,
		pageExecutionVersion,
		pageExecutionRevision,
		surfaceIdentityKey,
	]);
	const loadEventExecutionKeyRef = useRef(loadEventExecutionKey);
	loadEventExecutionKeyRef.current = loadEventExecutionKey;

	useEffect(() => {
		let cancelled = false;

		if (
			!cacheEnabled ||
			!surfaceIdentity ||
			!surfaceIdentityKey ||
			!page?.onLoadEventId
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
	}, [cacheEnabled, surfaceIdentity, surfaceIdentityKey, page?.onLoadEventId]);

	// Load the route content when dialog opens
	useEffect(() => {
		let cancelled = false;

		if (!appId || !dialog.route) {
			setIsLoading(false);
			setError("Missing app ID or route");
			return;
		}

		const loadContent = async () => {
			setIsLoading(true);
			setError(null);
			setPage(null);
			setPageRevision(null);
			setPageExecutionRevision(null);
			setRouteEvent(null);
			setSurface(null);
			try {
				// One authenticated read resolves route, Event, the exact served page and its
				// revisions, so a canary viewer's dialog renders the same variant as the host page.
				const bootstrap = await backend.pageState.getPageBootstrap(
					appId,
					dialog.route,
				);
				if (cancelled) return;
				if (bootstrap.routeMiss) {
					setError(`Route not found: ${dialog.route}`);
					return;
				}
				if (!bootstrap.page || !bootstrap.event.default_page_id) {
					setError("Route event does not have a page target");
					return;
				}
				if (!bootstrap.executionRevision) {
					setError(
						"This Page could not load its execution authorization. Reload and try again.",
					);
					return;
				}

				setRouteEvent(bootstrap.event);
				setPage(bootstrap.page);
				setPageRevision(bootstrap.revision ?? null);
				setPageExecutionRevision(bootstrap.executionRevision);
				setSurface(buildSurfaceFromPage(bootstrap.page, bootstrap.page.id));
			} catch (e) {
				if (cancelled) return;
				console.error("Failed to load dialog content:", e);
				setError("Failed to load content");
			} finally {
				if (!cancelled) setIsLoading(false);
			}
		};

		void loadContent();
		return () => {
			cancelled = true;
		};
	}, [appId, dialog.route, backend.pageState]);

	const handleServerMessage = useCallback((message: A2UIServerMessage) => {
		console.log("[RouteDialog] Server message:", message);
		if (message.type === "showScreen" || shouldRevealProgressively(message)) {
			setIsScreenRevealed(true);
		}
		setSurface((prevSurface) =>
			prevSurface ? applyA2UIMessage(prevSurface, message) : prevSurface,
		);
	}, []);

	// Use ref to access current surface without creating dependency cycles
	const surfaceRef = useRef(surface);
	useEffect(() => {
		surfaceRef.current = surface;
	}, [surface]);

	const elementSource = useCallback((): ElementSource | null => {
		const currentSurface = surfaceRef.current;
		if (!currentSurface) return null;
		return {
			surfaceId: currentSurface.id,
			components: currentSurface.components,
			storedValues: {},
		};
	}, []);

	// Save opted-in page surfaces after onLoad completes
	useEffect(() => {
		if (!surfaceIdentity || !surface || isLoadEventRunning) return;
		if (!cacheEnabled || !page?.onLoadEventId) return;
		if (
			!loadEventExecutionKey ||
			completedLoadEventKey !== loadEventExecutionKey
		)
			return;
		void writePageSurfaceCache(surfaceIdentity, surface);
	}, [
		surfaceIdentity,
		cacheEnabled,
		page?.onLoadEventId,
		surface,
		isLoadEventRunning,
		loadEventExecutionKey,
		completedLoadEventKey,
	]);

	// Execute onLoad event for dialog page
	useEffect(() => {
		const executeOnLoadEvent = async () => {
			if (!page?.onLoadEventId || !appId || !loadEventExecutionKey) {
				loadEventExecutedRef.current = null;
				setCompletedLoadEventKey(null);
				setIsLoadEventRunning(false);
				return;
			}

			if (!routeEvent?.id || !pageExecutionRevision) {
				console.warn(
					"[RouteDialog] Missing governed Page context for onLoad event",
				);
				return;
			}

			const executionKey = loadEventExecutionKey;
			if (loadEventExecutedRef.current === executionKey) return;
			loadEventExecutedRef.current = executionKey;

			setCompletedLoadEventKey(null);
			setIsScreenRevealed(false);
			setIsLoadEventRunning(true);

			try {
				const currentSurface = surfaceRef.current;
				const surfaceElements = currentSurface
					? await collectRunElements({
							backend,
							appId,
							boardId: undefined,
							surfaceId: currentSurface.id,
							components: currentSurface.components,
							storedValues: {},
						})
					: {};
				if (
					loadEventExecutionKeyRef.current !== executionKey ||
					loadEventExecutedRef.current !== executionKey
				)
					return;

				const payload = {
					id: "page_load",
					payload: {
						_elements: surfaceElements,
						_elements_mode: "demand",
						_route: dialog.route,
						_query_params: dialog.queryParams || {},
						_page_id: page.id,
						_dialog_id: dialog.id,
					},
				};

				const execFn =
					executionService?.executeEvent ?? backend.eventState.executeEvent;
				await execFn(
					appId,
					routeEvent.id,
					payload,
					false,
					undefined,
					(events) => {
						if (
							loadEventExecutionKeyRef.current !== executionKey ||
							loadEventExecutedRef.current !== executionKey
						)
							return;
						for (const event of events) {
							if (event.event_type === "a2ui") {
								if (handleWidgetQueryMessage(event.payload)) continue;
								if (handleElementsRequestMessage(event.payload, elementSource))
									continue;
								handleServerMessage(event.payload as A2UIServerMessage);
							}
						}
					},
					undefined,
					{
						kind: "special",
						specialEvent: "load",
						manifestRevision: pageExecutionRevision,
					},
				);
			} catch (e) {
				console.error("[RouteDialog] Failed to execute onLoad event:", e);
			} finally {
				if (loadEventExecutedRef.current === executionKey) {
					setCompletedLoadEventKey(executionKey);
					setIsLoadEventRunning(false);
				}
			}
		};

		if (!isLoading && page) {
			executeOnLoadEvent();
		}
	}, [
		appId,
		page,
		pageExecutionRevision,
		routeEvent?.id,
		loadEventExecutionKey,
		dialog,
		isLoading,
		backend,
		executionService,
		handleServerMessage,
		elementSource,
	]);

	const activeSurface =
		cachedSurface && isLoadEventRunning && !isScreenRevealed
			? cachedSurface
			: (surface ?? cachedSurface);
	const canRenderFromCache = Boolean(cachedSurface);
	const showLoading =
		(isLoading && !canRenderFromCache) ||
		isCacheLoading ||
		(isLoadEventRunning && !canRenderFromCache && !isScreenRevealed);
	const renderError =
		error ??
		(isGovernedPage && !pageExecutionRevision
			? "This Page could not load its execution authorization. Reload and try again."
			: null);

	return (
		<Dialog open={dialog.isOpen} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-4xl max-h-[90vh] overflow-auto">
				{dialog.title && (
					<DialogHeader>
						<DialogTitle>{dialog.title}</DialogTitle>
					</DialogHeader>
				)}
				<div className="min-h-50">
					{showLoading && <PageLoadingSkeleton className="h-48" />}
					{renderError && !showLoading && (
						<div className="flex items-center justify-center h-48 text-muted-foreground">
							<p>{renderError}</p>
						</div>
					)}
					{!showLoading && !renderError && activeSurface && (
						<A2UIRenderer
							surface={activeSurface}
							widgetRefs={page?.widgetRefs}
							appId={appId}
							boardId={pageExecutionBoardId}
							boardVersion={pageExecutionVersion}
							eventId={routeEvent?.id}
							governedPage={isGovernedPage}
							onA2UIMessage={handleServerMessage}
							isPreviewMode={true}
							openDialog={openDialog}
							closeDialog={closeDialog}
						/>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
}

// Helper function to build surface from page
function buildSurfaceFromPage(page: IPage, surfaceId: string): Surface | null {
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
		id: surfaceId,
		rootComponentId,
		components: componentsRecord,
		canvasSettings: page.canvasSettings,
	};
}
