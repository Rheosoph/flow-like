"use client";

import { usePathname, useRouter } from "next/navigation";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import { appGlobalState, pageLocalState } from "../../lib/idb-storage";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import { useBackend } from "../../state/backend-state";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import { useRouteDialogSafe } from "./RouteDialogProvider";
import { useWidgetInstance } from "./layout/A2UIWidgetInstance";
import type {
	A2UIClientMessage,
	A2UIServerMessage,
	Action,
	SurfaceComponent,
} from "./types";

type ActionHandler = (message: A2UIClientMessage) => void;
type A2UIMessageHandler = (message: A2UIServerMessage) => void;

function toBoundValue(value: unknown): Record<string, unknown> {
	if (typeof value === "boolean") return { literalBool: value };
	if (typeof value === "number") return { literalNumber: value };
	if (typeof value === "string") return { literalString: value };
	if (Array.isArray(value)) return { literalString: JSON.stringify(value) };
	if (value === null || value === undefined) return { literalString: "" };
	return { literalString: JSON.stringify(value) };
}

interface ActionContextValue {
	onAction?: ActionHandler;
	onA2UIMessage?: A2UIMessageHandler;
	surfaceId: string;
	appId?: string;
	boardId?: string;
	components?: Record<string, SurfaceComponent>;
	globalState: Record<string, unknown>;
	pageState: Record<string, unknown>;
	setGlobalState: (key: string, value: unknown) => void;
	setPageState: (key: string, value: unknown) => void;
	clearPageState: () => void;
	isPreviewMode: boolean;
	openDialog?: (
		route: string,
		title?: string,
		queryParams?: Record<string, string>,
		dialogId?: string,
	) => void;
	closeDialog?: (dialogId?: string) => void;
	getElementValues: () => Record<string, unknown>;
}

const ActionContext = createContext<ActionContextValue | null>(null);

interface ActionProviderProps {
	onAction?: ActionHandler;
	onA2UIMessage?: A2UIMessageHandler;
	surfaceId: string;
	appId?: string;
	boardId?: string;
	components?: Record<string, SurfaceComponent>;
	children: ReactNode;
	isPreviewMode?: boolean;
	openDialog?: (
		route: string,
		title?: string,
		queryParams?: Record<string, string>,
		dialogId?: string,
	) => void;
	closeDialog?: (dialogId?: string) => void;
}

export function ActionProvider({
	onAction,
	onA2UIMessage,
	surfaceId,
	appId,
	boardId,
	components,
	children,
	isPreviewMode = false,
	openDialog: openDialogProp,
	closeDialog: closeDialogProp,
}: ActionProviderProps) {
	const pathname = usePathname();
	const routeDialog = useRouteDialogSafe();
	const [globalState, setGlobalStateMap] = useState<Record<string, unknown>>(
		{},
	);
	const pageStateRef = useRef<Record<string, Record<string, unknown>>>({});
	const [pageState, setPageStateLocal] = useState<Record<string, unknown>>({});
	const [isStateLoaded, setIsStateLoaded] = useState(false);

	// Use props if provided (for portal'd dialogs), otherwise use context
	const openDialog = openDialogProp ?? routeDialog?.openDialog;
	const closeDialog = closeDialogProp ?? routeDialog?.closeDialog;

	// In-memory storage for element values (current page only)
	const elementValuesRef = useRef<Record<string, unknown>>({});

	// Getter for element values (used by useExecuteAction)
	const getElementValues = useCallback(() => {
		console.log(
			"[ActionHandler] getElementValues called, current values:",
			elementValuesRef.current,
		);
		return elementValuesRef.current;
	}, []);

	// Wrap onAction to intercept change events and store element values in memory
	const wrappedOnAction = useCallback(
		(message: A2UIClientMessage) => {
			// Store element values on change actions
			if (message.name === "change" && message.sourceComponentId) {
				const elementId = `${message.surfaceId}/${message.sourceComponentId}`;
				const value = message.context?.value ?? message.context?.checked;

				console.log("[ActionHandler] Storing element value:", {
					elementId,
					value,
					surfaceId: message.surfaceId,
				});

				// Store in memory
				elementValuesRef.current[elementId] = value;
			}

			// Forward to original handler
			onAction?.(message);
		},
		[onAction],
	);

	// Load persisted state from IndexedDB on mount
	useEffect(() => {
		if (!appId) {
			setIsStateLoaded(true);
			return;
		}

		const loadPersistedState = async () => {
			try {
				// Load global state
				const persistedGlobal = await appGlobalState.getAll(appId);
				if (Object.keys(persistedGlobal).length > 0) {
					setGlobalStateMap(persistedGlobal);
				}

				// Load page state for current page
				const pageId = pathname || "default";
				const persistedPage = await pageLocalState.getAll(appId, pageId);
				if (Object.keys(persistedPage).length > 0) {
					pageStateRef.current[pageId] = persistedPage;
					setPageStateLocal(persistedPage);
				}
			} catch (error) {
				console.error("Failed to load persisted state:", error);
			} finally {
				setIsStateLoaded(true);
			}
		};

		loadPersistedState();
	}, [appId, pathname]);

	// Load page state when pathname changes
	useEffect(() => {
		if (!appId || !isStateLoaded) return;

		const pageId = pathname || "default";

		// Check if we already have this page's state in memory
		if (pageStateRef.current[pageId]) {
			setPageStateLocal(pageStateRef.current[pageId]);
			return;
		}

		// Load from IndexedDB
		const loadPageState = async () => {
			try {
				const persistedPage = await pageLocalState.getAll(appId, pageId);
				pageStateRef.current[pageId] = persistedPage;
				setPageStateLocal(persistedPage);
			} catch (error) {
				console.error("Failed to load page state:", error);
				pageStateRef.current[pageId] = {};
				setPageStateLocal({});
			}
		};

		loadPageState();
	}, [appId, pathname, isStateLoaded]);

	const setGlobalState = useCallback(
		(key: string, value: unknown) => {
			setGlobalStateMap((prev) => {
				const next = { ...prev, [key]: value };
				// Persist to IndexedDB
				if (appId) {
					appGlobalState
						.set(appId, key, value)
						.catch((err) =>
							console.error("Failed to persist global state:", err),
						);
				}
				return next;
			});
		},
		[appId],
	);

	const setPageState = useCallback(
		(key: string, value: unknown) => {
			const pageId = pathname || "default";
			if (!pageStateRef.current[pageId]) {
				pageStateRef.current[pageId] = {};
			}
			pageStateRef.current[pageId][key] = value;
			setPageStateLocal({ ...pageStateRef.current[pageId] });

			// Persist to IndexedDB
			if (appId) {
				pageLocalState
					.set(appId, pageId, key, value)
					.catch((err) => console.error("Failed to persist page state:", err));
			}
		},
		[pathname, appId],
	);

	const clearPageState = useCallback(() => {
		const pageId = pathname || "default";
		pageStateRef.current[pageId] = {};
		setPageStateLocal({});

		// Clear from IndexedDB
		if (appId) {
			pageLocalState
				.clearPage(appId, pageId)
				.catch((err) => console.error("Failed to clear page state:", err));
		}
	}, [pathname, appId]);

	// Wrap onA2UIMessage to handle state updates
	const handleA2UIMessage = useCallback(
		(message: A2UIServerMessage) => {
			switch (message.type) {
				case "setGlobalState": {
					const { key, value } = message as { key: string; value: unknown };
					setGlobalState(key, value);
					break;
				}
				case "setPageState": {
					const { pageId, key, value } = message as {
						pageId: string;
						key: string;
						value: unknown;
					};
					const currentPageId = pathname || "default";
					// Only apply if it's for the current page
					if (pageId === currentPageId) {
						setPageState(key, value);
					} else {
						// Store for other pages in memory
						if (!pageStateRef.current[pageId]) {
							pageStateRef.current[pageId] = {};
						}
						pageStateRef.current[pageId][key] = value;
						// Also persist to IndexedDB for cross-page state
						if (appId) {
							pageLocalState
								.set(appId, pageId, key, value)
								.catch((err) =>
									console.error(
										"Failed to persist page state for other page:",
										err,
									),
								);
						}
					}
					break;
				}
				case "clearPageState": {
					const { pageId } = message as { pageId: string };
					pageStateRef.current[pageId] = {};
					if (pageId === (pathname || "default")) {
						setPageStateLocal({});
					}
					// Also clear from IndexedDB
					if (appId) {
						pageLocalState
							.clearPage(appId, pageId)
							.catch((err) =>
								console.error("Failed to clear page state:", err),
							);
					}
					break;
				}
				case "clearFileInput": {
					const { surfaceId: targetSurfaceId, componentId } = message as {
						surfaceId: string;
						componentId: string;
					};
					window.dispatchEvent(
						new CustomEvent("a2ui:clearFileInput", {
							detail: { surfaceId: targetSurfaceId, componentId },
						}),
					);
					break;
				}
				default:
					// Forward to original handler
					onA2UIMessage?.(message);
			}
		},
		[onA2UIMessage, pathname, appId, setGlobalState, setPageState],
	);

	return (
		<ActionContext.Provider
			value={{
				onAction: wrappedOnAction,
				onA2UIMessage: handleA2UIMessage,
				surfaceId,
				appId,
				boardId,
				components,
				globalState,
				pageState,
				setGlobalState,
				setPageState,
				clearPageState,
				isPreviewMode,
				openDialog,
				closeDialog,
				getElementValues,
			}}
		>
			{children}
		</ActionContext.Provider>
	);
}

export function useActionContext() {
	const context = useContext(ActionContext);
	if (!context) {
		return {
			appId: undefined,
			boardId: undefined,
			surfaceId: "",
			isPreviewMode: false,
		};
	}
	return {
		appId: context.appId,
		boardId: context.boardId,
		surfaceId: context.surfaceId,
		isPreviewMode: context.isPreviewMode,
	};
}

/**
 * Hook to get the onAction handler from context.
 * This ensures actions go through the wrapper that stores element values.
 * Use this instead of the onAction prop for interactive components.
 */
export function useOnAction() {
	const context = useContext(ActionContext);
	return context?.onAction;
}

export function useActions() {
	const context = useContext(ActionContext);
	if (!context) {
		throw new Error("useActions must be used within an ActionProvider");
	}

	const { onAction, surfaceId, isPreviewMode } = context;

	const trigger = useCallback(
		(
			action: Action | string,
			additionalContext: Record<string, unknown> = {},
		) => {
			// Only trigger actions in preview mode
			if (!isPreviewMode || !onAction) return;

			const actionObj =
				typeof action === "string" ? { name: action, context: {} } : action;

			const message: A2UIClientMessage = {
				type: "userAction",
				name: actionObj.name,
				surfaceId,
				sourceComponentId: "",
				timestamp: Date.now(),
				context: { ...actionObj.context, ...additionalContext },
			};

			onAction(message);
		},
		[surfaceId, onAction, isPreviewMode],
	);

	return { trigger, isPreviewMode };
}

export function useExecuteAction() {
	const router = useRouter();
	const pathname = usePathname();
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const widgetInstance = useWidgetInstance();
	const {
		onAction,
		onA2UIMessage,
		surfaceId,
		appId,
		boardId,
		components,
		globalState,
		pageState,
		isPreviewMode,
		openDialog,
		closeDialog,
		getElementValues,
	} = useContext(ActionContext) ?? {};

	// Cache for execution elements per board
	const executionElementsCache = useRef<Map<string, Record<string, unknown>>>(
		new Map(),
	);

	const handleA2UIEvents = useCallback(
		(events: IIntercomEvent[]) => {
			console.log("[A2UI] Received events from backend:", events);

			for (const event of events) {
				console.log(
					"[A2UI] Processing event:",
					event.event_type,
					event.payload,
				);

				if (event.event_type === "a2ui") {
					const message = event.payload as A2UIServerMessage;
					console.log("[A2UI] A2UI message:", message);

					// Handle navigation directly - ActionHandler handles this, don't duplicate in page-interface
					if (message.type === "navigateTo") {
						const { route, replace, queryParams } = message as {
							route: string;
							replace: boolean;
							queryParams?: Record<string, string>;
						};

						// Build the navigation URL using query params format
						let navUrl = route;

						// If route doesn't start with /use and is an internal route, build query params URL
						if (
							appId &&
							!route.startsWith("/use") &&
							!route.startsWith("http")
						) {
							// Parse any query params that might be in the route itself
							const [routePath, routeQueryString] = route.split("?");
							const params = new URLSearchParams();
							params.set("id", appId);
							params.set("route", routePath);

							// Add query params from the route string (e.g., /new?foo=bar)
							if (routeQueryString) {
								const routeParams = new URLSearchParams(routeQueryString);
								routeParams.forEach((value, key) => {
									params.set(key, value);
								});
							}

							// Add additional query params if provided (these override route params)
							if (queryParams) {
								for (const [key, value] of Object.entries(queryParams)) {
									params.set(key, value);
								}
							}
							navUrl = `/use?${params.toString()}`;
						} else if (queryParams && Object.keys(queryParams).length > 0) {
							// External or already-formed URL with additional query params
							const params = new URLSearchParams(queryParams);
							const separator = navUrl.includes("?") ? "&" : "?";
							navUrl = `${navUrl}${separator}${params.toString()}`;
						}

						console.log(
							"[A2UI] Navigating to:",
							navUrl,
							"replace:",
							replace,
							"appId:",
							appId,
							"currentPath:",
							pathname,
						);

						// Use window.location for navigation in Tauri/desktop environment
						// This ensures the navigation actually works regardless of React/Next.js state
						if (replace) {
							window.location.replace(navUrl);
						} else {
							window.location.href = navUrl;
						}
						// Continue to next event, navigation is fully handled here
						continue;
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

						console.log("[A2UI] Setting query param:", key, "=", value);

						if (replace) {
							router.replace(url.pathname + url.search);
						} else {
							router.push(url.pathname + url.search);
						}
						continue;
					}

					// Handle open dialog
					if (message.type === "openDialog") {
						const { route, title, queryParams, dialogId } = message as {
							route: string;
							title?: string;
							queryParams?: Record<string, string>;
							dialogId?: string;
						};

						console.log("[A2UI] openDialog message received:", {
							route,
							title,
							queryParams,
							dialogId,
							openDialogFn: !!openDialog,
						});

						if (openDialog) {
							console.log(
								"[A2UI] Calling openDialog function with route:",
								route,
							);
							openDialog(route, title, queryParams, dialogId);
						} else {
							console.warn(
								"[A2UI] openDialog not available, cannot open dialog. Make sure ActionProvider is inside RouteDialogProvider or openDialog prop is passed.",
							);
						}
						continue;
					}

					// Handle close dialog
					if (message.type === "closeDialog") {
						const { dialogId } = message as { dialogId?: string };

						console.log("[A2UI] closeDialog message received:", { dialogId });

						if (closeDialog) {
							closeDialog(dialogId);
						} else {
							console.warn(
								"[A2UI] closeDialog not available, cannot close dialog",
							);
						}
						continue;
					}

					// Forward other A2UI messages to the handler (state updates, element updates, etc.)
					if (onA2UIMessage) {
						console.log(
							"[A2UI] Forwarding message to handler:",
							message.type,
							message,
						);
						onA2UIMessage(message);
					} else {
						console.warn("[A2UI] No onA2UIMessage handler available!");
					}
				}
			}
		},
		[router, onA2UIMessage, appId, openDialog, closeDialog],
	);

	const executeAction = useCallback(
		async (action: Action | undefined) => {
			// Only execute actions in preview mode
			if (!isPreviewMode || !action) return;

			const { name, context } = action;

			console.log("[ActionHandler] executeAction:", {
				name,
				context,
				appId,
				isPreviewMode,
			});

			switch (name) {
				case "navigate_page": {
					const route = context.route as string | undefined;
					const queryParamsRaw = context.queryParams as
						| string
						| Record<string, string>
						| undefined;

					// Parse queryParams if it's a JSON string
					let extraParams: Record<string, string> = {};
					if (typeof queryParamsRaw === "string" && queryParamsRaw.trim()) {
						try {
							extraParams = JSON.parse(queryParamsRaw);
						} catch {
							console.warn(
								"[ActionHandler] Invalid queryParams JSON:",
								queryParamsRaw,
							);
						}
					} else if (typeof queryParamsRaw === "object" && queryParamsRaw) {
						extraParams = queryParamsRaw;
					}

					console.log("[ActionHandler] navigate_page:", {
						route,
						appId,
						extraParams,
					});
					if (route) {
						// Build query params URL for internal routes
						if (
							appId &&
							!route.startsWith("/use") &&
							!route.startsWith("http")
						) {
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

							// Add extra query params (these override route params)
							for (const [key, value] of Object.entries(extraParams)) {
								params.set(key, value);
							}
							const navUrl = `/use?${params.toString()}`;
							console.log("[ActionHandler] Navigating to:", navUrl);
							router.push(navUrl);
						} else {
							console.log("[ActionHandler] Fallback navigation to:", route);
							router.push(route);
						}
					}
					break;
				}
				case "external_link": {
					const url = context.url as string | undefined;
					if (url) {
						window.open(url, "_blank", "noopener,noreferrer");
					}
					break;
				}
				case "workflow_event": {
					const nodeId = context.nodeId as string | undefined;
					const actionBoardId = context.boardId as string | undefined;
					const boardVersion = context.boardVersion as
						| [number, number, number]
						| undefined;
					const contextAppId = context.appId as string | undefined;

					const effectiveAppId = contextAppId || appId;
					const effectiveBoardId = actionBoardId || boardId;

					if (nodeId && effectiveBoardId && effectiveAppId) {
						try {
							// Get cached execution elements or fetch them
							const cacheKey = `${effectiveBoardId}:${surfaceId}`;
							let elementsMap = executionElementsCache.current.get(cacheKey);

							if (!elementsMap) {
								// Fetch required elements from backend
								try {
									elementsMap = await backend.boardState.getExecutionElements(
										effectiveAppId,
										effectiveBoardId,
										surfaceId || "",
										false, // wildcard = false, only get required elements
									);

									// If no specific elements returned, fall back to all components
									if (!elementsMap || Object.keys(elementsMap).length === 0) {
										elementsMap = {};
										if (components && surfaceId) {
											for (const [id, comp] of Object.entries(components)) {
												elementsMap[`${surfaceId}/${id}`] = comp;
											}
										}
									}

									// Cache for subsequent executions
									executionElementsCache.current.set(cacheKey, elementsMap);
								} catch (err) {
									console.warn(
										"[A2UI] Failed to fetch execution elements, falling back to all components:",
										err,
									);
									// Fall back to all components
									elementsMap = {};
									if (components && surfaceId) {
										for (const [id, comp] of Object.entries(components)) {
											elementsMap[`${surfaceId}/${id}`] = comp;
										}
									}
								}
							}

							// Merge in-memory element values (user input state)
							const storedValues = getElementValues?.() ?? {};
							console.log("[A2UI] workflow_event merging elements:", {
								elementsMapKeys: Object.keys(elementsMap),
								storedValuesKeys: Object.keys(storedValues),
								storedValues,
							});

							const mergedElements: Record<string, unknown> = {};
							for (const [elementId, element] of Object.entries(elementsMap)) {
								const storedValue = storedValues[elementId];
								console.log("[A2UI] Checking element:", {
									elementId,
									hasStoredValue: storedValue !== undefined,
									storedValue,
								});
								if (storedValue !== undefined) {
									const comp = element as Record<string, unknown>;
									const componentData = comp.component as
										| Record<string, unknown>
										| undefined;
									if (componentData) {
										mergedElements[elementId] = {
											...comp,
											component: {
												...componentData,
												value: toBoundValue(storedValue),
											},
										};
									} else {
										mergedElements[elementId] = element;
									}
								} else {
									mergedElements[elementId] = element;
								}
							}

							const currentPageId = pathname || "default";

							// Extract query params from the current URL
							const queryParams: Record<string, string> = {};
							if (typeof window !== "undefined") {
								const searchParams = new URLSearchParams(
									window.location.search,
								);
								searchParams.forEach((value, key) => {
									queryParams[key] = value;
								});
							}

							const payload = {
								id: nodeId,
								payload: {
									_elements: mergedElements,
									_route:
										typeof window !== "undefined"
											? window.location.pathname
											: "",
									_query_params: queryParams,
									_page_id: currentPageId,
									_global_state: globalState || {},
									_page_state: pageState || {},
								},
								version: boardVersion,
							};

							// Use execution service if available (checks runtime variables)
							const execFn =
								executionService?.executeBoard ??
								backend.boardState.executeBoard;
							await execFn(
								effectiveAppId,
								effectiveBoardId,
								payload,
								false, // streamState
								undefined, // eventId
								handleA2UIEvents, // callback for A2UI events
							);
						} catch (error) {
							console.error("Failed to execute workflow event:", error);
						}
					} else {
						console.warn("Missing required context for workflow_event:", {
							nodeId,
							boardId: effectiveBoardId,
							appId: effectiveAppId,
						});
					}
					break;
				}
			case "widget_event": {
					console.log("[A2UI] widget_event triggered:", { context, widgetInstance, appId, boardId });
					const actionId = context.actionId as string | undefined;
					if (!actionId) {
						console.warn("[A2UI] widget_event missing actionId");
						break;
					}

					// Look up the binding from the widget instance's action bindings
					const binding = widgetInstance?.actionBindings[actionId];
					if (!binding) {
						console.warn("[A2UI] widget_event: no binding found for actionId:", actionId, "available bindings:", widgetInstance?.actionBindings);
						break;
					}

					if (!("workflow" in binding)) {
						console.warn("[A2UI] widget_event: only workflow bindings are supported for execution, got:", binding);
						break;
					}

					const nodeId = binding.workflow.flowId;

					const effectiveAppId = appId;
					const effectiveBoardId = boardId;

					if (effectiveBoardId && effectiveAppId) {
						try {
							const cacheKey = `${effectiveBoardId}:${surfaceId}`;
							let elementsMap = executionElementsCache.current.get(cacheKey);

							if (!elementsMap) {
								try {
									elementsMap = await backend.boardState.getExecutionElements(
										effectiveAppId,
										effectiveBoardId,
										surfaceId || "",
										false,
									);

									if (!elementsMap || Object.keys(elementsMap).length === 0) {
										elementsMap = {};
										if (components && surfaceId) {
											for (const [id, comp] of Object.entries(components)) {
												elementsMap[`${surfaceId}/${id}`] = comp;
											}
										}
									}

									executionElementsCache.current.set(cacheKey, elementsMap);
								} catch (err) {
									console.warn("[A2UI] Failed to fetch execution elements for widget_event:", err);
									elementsMap = {};
									if (components && surfaceId) {
										for (const [id, comp] of Object.entries(components)) {
											elementsMap[`${surfaceId}/${id}`] = comp;
										}
									}
								}
							}

							const storedValues = getElementValues?.() ?? {};
							const mergedElements: Record<string, unknown> = {};
							for (const [elementId, element] of Object.entries(elementsMap)) {
								const storedValue = storedValues[elementId];
								if (storedValue !== undefined) {
									const comp = element as Record<string, unknown>;
									const componentData = comp.component as Record<string, unknown> | undefined;
									if (componentData) {
										mergedElements[elementId] = {
											...comp,
											component: {
												...componentData,
												value: toBoundValue(storedValue),
											},
										};
									} else {
										mergedElements[elementId] = element;
									}
								} else {
									mergedElements[elementId] = element;
								}
							}

							const payload = {
								id: nodeId,
								payload: {
									_elements: mergedElements,
									_widget_instance_id: widgetInstance?.instanceId ?? "",
									_action_id: actionId,
									_action_context: context,
								},
							};

							const execFn = executionService?.executeBoard ?? backend.boardState.executeBoard;
							await execFn(
								effectiveAppId,
								effectiveBoardId,
								payload,
								false,
								undefined,
								handleA2UIEvents,
							);
						} catch (error) {
							console.error("[A2UI] Failed to execute widget event:", error);
						}
					} else {
						console.warn("[A2UI] Missing appId or boardId for widget_event:", { appId: effectiveAppId, boardId: effectiveBoardId });
					}
					break;
				}
				default:
					if (onAction) {
						onAction({
							type: "userAction",
							name,
							surfaceId: surfaceId ?? "",
							sourceComponentId: "",
							timestamp: Date.now(),
							context,
						});
					}
			}
		},
		[
			router,
			pathname,
			backend,
			executionService,
			onAction,
			surfaceId,
			appId,
			boardId,
			components,
			globalState,
			pageState,
			handleA2UIEvents,
			isPreviewMode,
			widgetInstance,
		],
	);

	return { executeAction, isPreviewMode: isPreviewMode ?? false };
}
