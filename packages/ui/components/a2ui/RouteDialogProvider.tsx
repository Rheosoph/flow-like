"use client";

import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	readPageSurfaceCache,
	writePageSurfaceCache,
} from "../../lib/page-surface-cache";
import type { IEvent } from "../../lib/schema/flow/event";
import { useBackend } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import { PageLoadingSkeleton } from "../interfaces/page-loading-skeleton";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import { A2UIRenderer } from "./A2UIRenderer";
import { normalizeBoxes, resolveBoxesField } from "./bbox-utils";
import { applyMediaSourceUpdate } from "./media-source";
import type { A2UIServerMessage, Surface, SurfaceComponent } from "./types";

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
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const [isLoading, setIsLoading] = useState(true);
	const [isLoadEventRunning, setIsLoadEventRunning] = useState(false);
	const [isCacheLoading, setIsCacheLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [surface, setSurface] = useState<Surface | null>(null);
	const [page, setPage] = useState<IPage | null>(null);
	const [routeMapping, setRouteMapping] = useState<IRouteMapping | null>(null);
	const [routeEvent, setRouteEvent] = useState<IEvent | null>(null);
	const loadEventExecutedRef = useRef<string | null>(null);
	const [cachedSurface, setCachedSurface] = useState<Surface | null>(null);

	useEffect(() => {
		let cancelled = false;

		if (!page?.cache || !appId || !page.id) {
			setCachedSurface(null);
			setIsCacheLoading(false);
			return;
		}

		setIsCacheLoading(true);
		void readPageSurfaceCache(appId, page)
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
	}, [appId, page?.cache, page?.id, page?.updatedAt]);

	// Load the route content when dialog opens
	useEffect(() => {
		if (!appId || !dialog.route) {
			setIsLoading(false);
			setError("Missing app ID or route");
			return;
		}

		const loadContent = async () => {
			setIsLoading(true);
			setError(null);
			try {
				// Get route mapping
				const mapping: IRouteMapping | null =
					await backend.routeState.getRouteByPath(appId, dialog.route);

				if (!mapping) {
					setError(`Route not found: ${dialog.route}`);
					setIsLoading(false);
					return;
				}

				setRouteMapping(mapping);

				// Get the event for this route
				const events = await backend.eventState.getEvents(appId);
				const event = events.find((e) => e.id === mapping.eventId);

				if (!event) {
					setError(`Event not found for route: ${dialog.route}`);
					setIsLoading(false);
					return;
				}

				setRouteEvent(event);

				// Check if event has a page target
				if (event.default_page_id) {
					const pageResult = await backend.pageState.getPage(
						appId,
						event.default_page_id,
						undefined,
					);

					if (pageResult) {
						setPage(pageResult);
						// Build surface from page
						const builtSurface = buildSurfaceFromPage(
							pageResult,
							pageResult.id,
						);
						setSurface(builtSurface);
					} else {
						setError(`Page not found: ${event.default_page_id}`);
					}
				} else {
					setError("Route event does not have a page target");
				}
			} catch (e) {
				console.error("Failed to load dialog content:", e);
				setError("Failed to load content");
			} finally {
				setIsLoading(false);
			}
		};

		loadContent();
	}, [
		appId,
		dialog.route,
		backend.routeState,
		backend.pageState,
		backend.eventState,
	]);

	const handleServerMessage = useCallback((message: A2UIServerMessage) => {
		console.log("[RouteDialog] Server message:", message);

		if (message.type === "setCanvasSettings") {
			setSurface((prevSurface) => {
				if (!prevSurface || message.surfaceId !== prevSurface.id) {
					return prevSurface;
				}

				// Filter null/undefined values to avoid overwriting existing settings
				// (Rust serializes Option::None as null)
				const filtered = Object.fromEntries(
					Object.entries(message.canvasSettings).filter(([, v]) => v != null),
				);

				return {
					...prevSurface,
					canvasSettings: {
						...prevSurface.canvasSettings,
						...filtered,
					},
				};
			});
			return;
		}

		if (message.type === "dataModelUpdate") {
			setSurface((prevSurface) => {
				if (!prevSurface || message.surfaceId !== prevSurface.id) {
					return prevSurface;
				}

				const entries = new Map(
					(prevSurface.dataModel ?? []).map((entry) => [entry.path, entry]),
				);
				for (const entry of message.contents) {
					entries.set(entry.path, entry);
				}

				return {
					...prevSurface,
					dataModel: Array.from(entries.values()),
				};
			});
			return;
		}

		if (message.type !== "upsertElement") return;

		setSurface((prevSurface) => {
			if (!prevSurface) return prevSurface;

			const { element_id: elementId, value } = message;
			if (!elementId) return prevSurface;

			const [surfaceId, componentId] = elementId.includes("/")
				? elementId.split("/", 2)
				: [prevSurface.id, elementId];

			if (surfaceId !== prevSurface.id) return prevSurface;

			const component = prevSurface.components[componentId];
			if (!component) return prevSurface;

			const updateValue = value as Record<string, unknown>;
			const updateType = updateValue?.type as string;

			let updatedComponent: SurfaceComponent = { ...component };

			if (updateType === "setText") {
				const text = updateValue.text as string;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						text,
					} as unknown as SurfaceComponent["component"],
				};
			} else if (updateType === "setMediaSource") {
				updatedComponent = applyMediaSourceUpdate(component, updateValue);
			} else if (updateType === "setGeoMapViewport") {
				const viewport = updateValue.viewport as
					| { literalJson?: string }
					| undefined;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						viewport,
					} as unknown as SurfaceComponent["component"],
				};
			} else if (updateType === "setProps") {
				const props = updateValue.props as Record<string, unknown>;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						...props,
					} as unknown as SurfaceComponent["component"],
				};
			} else if (
				updateType === "setChartData" ||
				updateType === "setNivoData"
			) {
				const data = updateValue.data;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						data: { literalJson: JSON.stringify(data) },
					} as unknown as SurfaceComponent["component"],
				};
			} else if (
				updateType === "setChartLayout" ||
				updateType === "setNivoConfig"
			) {
				const configOrLayout = updateValue.layout ?? updateValue.config;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						...(updateValue.layout !== undefined && {
							layout: { literalJson: JSON.stringify(configOrLayout) },
						}),
						...(updateValue.config !== undefined && {
							config: { literalJson: JSON.stringify(configOrLayout) },
						}),
					} as unknown as SurfaceComponent["component"],
				};
			} else if (
				updateType === "setOverlayBoxes" ||
				updateType === "setLabelerBoxes"
			) {
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						boxes: { literalOptions: normalizeBoxes(updateValue.boxes) },
					} as unknown as SurfaceComponent["component"],
				};
			} else if (
				updateType === "addOverlayBox" ||
				updateType === "addLabelerBox"
			) {
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				const existing = resolveBoxesField(componentData.boxes);
				const added = normalizeBoxes([updateValue.box]);
				updatedComponent = {
					...component,
					component: {
						...componentData,
						boxes: { literalOptions: [...existing, ...added] },
					} as unknown as SurfaceComponent["component"],
				};
			} else if (updateType === "clearOverlayBoxes") {
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						boxes: { literalOptions: [] },
					} as unknown as SurfaceComponent["component"],
				};
			} else if (updateType === "removeLabelerBox") {
				const boxId = updateValue.boxId as string;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				const remaining = resolveBoxesField(componentData.boxes).filter(
					(box) => box.id !== boxId,
				);
				updatedComponent = {
					...component,
					component: {
						...componentData,
						boxes: { literalOptions: remaining },
					} as unknown as SurfaceComponent["component"],
				};
			} else if (updateType === "updateLabelerBoxLabel") {
				const boxId = updateValue.boxId as string;
				const label = updateValue.label as string;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				const updated = resolveBoxesField(componentData.boxes).map((box) =>
					box.id === boxId ? { ...box, label } : box,
				);
				updatedComponent = {
					...component,
					component: {
						...componentData,
						boxes: { literalOptions: updated },
					} as unknown as SurfaceComponent["component"],
				};
			} else if (updateType === "setLabelerImage") {
				const src = updateValue.src as string;
				const alt = updateValue.alt as string | undefined;
				const componentData = component.component as unknown as Record<
					string,
					unknown
				>;
				updatedComponent = {
					...component,
					component: {
						...componentData,
						src: { literalString: src },
						...(alt !== undefined ? { alt: { literalString: alt } } : {}),
					} as unknown as SurfaceComponent["component"],
				};
			}

			return {
				...prevSurface,
				components: {
					...prevSurface.components,
					[componentId]: updatedComponent,
				},
			};
		});
	}, []);

	// Use ref to access current surface without creating dependency cycles
	const surfaceRef = useRef(surface);
	useEffect(() => {
		surfaceRef.current = surface;
	}, [surface]);

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

	// Save surface to cache after onLoad completes
	useEffect(() => {
		if (!page?.cache || !appId || !page.id || !surface || isLoadEventRunning)
			return;
		void writePageSurfaceCache(appId, page, surface);
	}, [page?.cache, page?.id, appId, surface, isLoadEventRunning]);

	// Execute onLoad event for dialog page
	useEffect(() => {
		const executeOnLoadEvent = async () => {
			if (!page?.onLoadEventId || !appId) return;

			const boardId = page.boardId || routeEvent?.board_id;
			if (!boardId) {
				console.warn("[RouteDialog] No boardId available for onLoad event");
				return;
			}

			const executionKey = `${dialog.id}:${page.id}:${page.onLoadEventId}`;
			if (loadEventExecutedRef.current === executionKey) return;
			loadEventExecutedRef.current = executionKey;

			setIsLoadEventRunning(true);

			try {
				// Get component data from surface (for GetElement to work)
				const surfaceElements = getElementsFromSurface();

				const payload = {
					id: page.onLoadEventId,
					payload: {
						_elements: surfaceElements,
						_route: dialog.route,
						_query_params: dialog.queryParams || {},
						_page_id: page.id,
						_dialog_id: dialog.id,
					},
				};

				// Use execution service if available (checks runtime variables)
				const execFn =
					executionService?.executeBoard ?? backend.boardState.executeBoard;
				await execFn(appId, boardId, payload, false, undefined, (events) => {
					for (const event of events) {
						if (event.event_type === "a2ui") {
							handleServerMessage(event.payload as A2UIServerMessage);
						}
					}
				});
			} catch (e) {
				console.error("[RouteDialog] Failed to execute onLoad event:", e);
			} finally {
				setIsLoadEventRunning(false);
			}
		};

		if (!isLoading && page) {
			executeOnLoadEvent();
		}
	}, [
		appId,
		page,
		routeEvent,
		dialog,
		isLoading,
		backend.boardState,
		executionService,
		handleServerMessage,
		getElementsFromSurface,
	]);

	const activeSurface =
		page?.cache && cachedSurface && isLoadEventRunning
			? cachedSurface
			: (surface ?? cachedSurface);
	const canRenderFromCache = Boolean(page?.cache && cachedSurface);
	const showLoading =
		(isLoading && !canRenderFromCache) ||
		(Boolean(page?.cache) && isCacheLoading) ||
		(isLoadEventRunning && !canRenderFromCache);

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
					{error && !showLoading && (
						<div className="flex items-center justify-center h-48 text-muted-foreground">
							<p>{error}</p>
						</div>
					)}
					{!showLoading && !error && activeSurface && (
						<A2UIRenderer
							surface={activeSurface}
							widgetRefs={page?.widgetRefs}
							appId={appId}
							boardId={page?.boardId || routeEvent?.board_id}
							eventId={routeEvent?.id}
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

	const rootComponentId = componentsRecord["root"]
		? "root"
		: page.components[0]?.id || "";

	return {
		id: surfaceId,
		rootComponentId,
		components: componentsRecord,
		canvasSettings: page.canvasSettings,
	};
}
