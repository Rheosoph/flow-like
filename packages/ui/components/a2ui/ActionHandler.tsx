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
import { toast } from "sonner";
import { appGlobalState, pageLocalState } from "../../lib/idb-storage";
import { getCurrentPageContext } from "../../lib/page-context";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import { type IBoard, IExecutionMode } from "../../lib/schema/flow/board";
import {
	type BoardVersion,
	resolveEventBoardVersion,
	withBoardVersion,
} from "../../lib/schema/flow/board-version";
import {
	type ITemporaryUploadExecutionTarget,
	useBackend,
} from "../../state/backend-state";
import {
	prerunBoardKey,
	prerunSwr,
} from "../../state/backend-state/prerun-cache";
import { useExecutionServiceOptional } from "../../state/execution-service-context";
import { useRouteDialogSafe } from "./RouteDialogProvider";
import { resolveEventActions } from "./event-handlers";
import {
	resolveWidgetInstanceEventRoute,
	useWidgetInstance,
} from "./layout/A2UIWidgetInstance";
import { collectMicroWidgetValueKeys } from "./micro-widget-host";
import type {
	A2UIClientMessage,
	A2UIServerMessage,
	Action,
	EventHandlers,
	SurfaceComponent,
} from "./types";
import { handleWidgetQueryMessage } from "./widget-query-handler";
import {
	buildFrontendContextPayload,
	compactWorkflowPayload,
} from "./workflow-payload";

export {
	buildFrontendContextPayload,
	compactWorkflowPayload,
} from "./workflow-payload";

type ActionHandler = (message: A2UIClientMessage) => void;
type A2UIMessageHandler = (message: A2UIServerMessage) => void;
type ExecuteActionFn = (
	action: Action | undefined,
	triggeringComponentId?: string,
	additionalContext?: Record<string, unknown>,
) => Promise<void>;

function toBoundValue(value: unknown): Record<string, unknown> {
	if (typeof value === "boolean") return { literalBool: value };
	if (typeof value === "number") return { literalNumber: value };
	if (typeof value === "string") return { literalString: value };
	if (value === undefined) return { literalString: "" };
	if (value === null || Array.isArray(value) || typeof value === "object") {
		return { literalJson: JSON.stringify(value) };
	}
	return { literalString: String(value) };
}

/**
 * Flattens surface.components plus widget-instance inline definitions into a
 * single `surfaceId/componentId -> element` map. Widget instances expose their
 * children through `inlineWidgetDef.components`, so a plain Object.entries on
 * surface.components misses them and Get Element nodes can't resolve those ids.
 */
function flattenSurfaceComponentsForElements(
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	if (!components || !surfaceId) return result;

	for (const [id, comp] of Object.entries(components)) {
		result[`${surfaceId}/${id}`] = comp;

		const compData = (comp as unknown as Record<string, unknown>).component as
			| Record<string, unknown>
			| undefined;

		if (compData?.type !== "widgetInstance") continue;

		const inlineDef = compData.inlineWidgetDef as
			| {
					components?: Array<{
						id: string;
						component: unknown;
						style?: unknown;
					}>;
			  }
			| undefined;

		for (const inner of inlineDef?.components ?? []) {
			if (!inner?.id) continue;
			const key = `${surfaceId}/${inner.id}`;
			if (result[key] !== undefined) continue;
			result[key] = {
				id: inner.id,
				component: inner.component,
				style: inner.style,
			};
		}
	}

	return result;
}

function mergeStoredElementValues(
	elementsMap: Record<string, unknown>,
	storedValues: Record<string, unknown>,
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
): Record<string, unknown> {
	const flatComponents = flattenSurfaceComponentsForElements(
		components,
		surfaceId,
	);
	const mergedElements: Record<string, unknown> = { ...flatComponents };

	for (const [elementId, element] of Object.entries(elementsMap)) {
		mergedElements[elementId] = element;
	}

	for (const [elementId, element] of Object.entries(mergedElements)) {
		const storedValue = storedValues[elementId];
		if (storedValue === undefined) continue;
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
		}
	}

	// Micro widget value mirrors are keyed "{instanceId}/values" (not prefixed
	// with the surface id), so they need their own allowlist to survive the merge.
	const microValueKeys = collectMicroWidgetValueKeys(components);

	for (const [elementId, storedValue] of Object.entries(storedValues)) {
		if (mergedElements[elementId] !== undefined) continue;
		const isMicroValues = microValueKeys.has(elementId);
		if (!isMicroValues && !elementId.startsWith(`${surfaceId}/`)) continue;

		mergedElements[elementId] = {
			id: isMicroValues ? "values" : elementId.slice(`${surfaceId}/`.length),
			component: { value: toBoundValue(storedValue) },
		};
	}

	return mergedElements;
}

/** Stable empty state for provider-less consumers, so hook deps stay steady. */
const EMPTY_STATE: Record<string, unknown> = Object.freeze({});

function decodePinDefaultValue(defaultValue: unknown): unknown {
	if (!Array.isArray(defaultValue) || defaultValue.length === 0) {
		return undefined;
	}

	try {
		const jsonString = new TextDecoder("utf-8").decode(
			new Uint8Array(defaultValue),
		);
		return JSON.parse(jsonString);
	} catch {
		return undefined;
	}
}

function summarizeBoardElementRefs(board: IBoard) {
	const refs: Array<{
		nodeId: string;
		nodeName: string;
		pinId: string;
		pinName: string;
		defaultValue: unknown;
	}> = [];

	const collectNodeRefs = (node: Record<string, unknown>) => {
		const pins = (node.pins as Record<string, Record<string, unknown>>) ?? {};
		for (const pin of Object.values(pins)) {
			const pinName = pin.name;
			if (typeof pinName !== "string" || !pinName.startsWith("element_ref")) {
				continue;
			}

			refs.push({
				nodeId: String(node.id ?? ""),
				nodeName: String(node.name ?? ""),
				pinId: String(pin.id ?? ""),
				pinName,
				defaultValue: decodePinDefaultValue(pin.default_value),
			});
		}
	};

	for (const node of Object.values(board.nodes ?? {})) {
		collectNodeRefs(node as Record<string, unknown>);
	}

	for (const layer of Object.values(board.layers ?? {})) {
		const layerNodes = (layer as Record<string, unknown>).nodes as
			| Record<string, Record<string, unknown>>
			| undefined;
		for (const node of Object.values(layerNodes ?? {})) {
			collectNodeRefs(node);
		}
	}

	return refs;
}

interface ActionContextValue {
	onAction?: ActionHandler;
	onA2UIMessage?: A2UIMessageHandler;
	surfaceId: string;
	appId?: string;
	boardId?: string;
	boardVersion?: BoardVersion;
	eventId?: string;
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
	setElementValue: (elementId: string, value: unknown) => void;
	resolveTemporaryUploadTarget: (
		action?: Action,
	) => Promise<ITemporaryUploadExecutionTarget>;
	triggeringComponents: ReadonlySet<string>;
	markComponentTriggering: (componentId: string, on: boolean) => void;
}

const ActionContext = createContext<ActionContextValue | null>(null);

interface ActionProviderProps {
	onAction?: ActionHandler;
	onA2UIMessage?: A2UIMessageHandler;
	surfaceId: string;
	appId?: string;
	boardId?: string;
	boardVersion?: BoardVersion;
	eventId?: string;
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
	boardVersion,
	eventId,
	components,
	children,
	isPreviewMode = false,
	openDialog: openDialogProp,
	closeDialog: closeDialogProp,
}: ActionProviderProps) {
	const pathname = usePathname();
	const backend = useBackend();
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

	// Imperative setter for element values that bypasses the change-action
	// wrapper. Used by micro widgets to mirror their `value:changed` state
	// under the "{instanceId}/values" elements-payload key.
	const setElementValue = useCallback((elementId: string, value: unknown) => {
		if (!elementId) return;
		elementValuesRef.current[elementId] = value;
	}, []);

	// Ref-counted set of components that are currently triggering an async action.
	// Components consume this to render a loading state for the duration of their action.
	const triggeringCountsRef = useRef<Map<string, number>>(new Map());
	const [triggeringComponents, setTriggeringComponents] = useState<
		ReadonlySet<string>
	>(() => new Set());

	const markComponentTriggering = useCallback(
		(componentId: string, on: boolean) => {
			if (!componentId) return;
			const counts = triggeringCountsRef.current;
			const current = counts.get(componentId) ?? 0;
			const next = on ? current + 1 : Math.max(0, current - 1);
			if (next === 0) {
				counts.delete(componentId);
			} else {
				counts.set(componentId, next);
			}
			setTriggeringComponents(new Set(counts.keys()));
		},
		[],
	);

	const resolveTemporaryUploadTarget = useCallback(
		async (action?: Action): Promise<ITemporaryUploadExecutionTarget> => {
			// Browser backends always dispatch remotely. Keeping this explicit also
			// prevents a permissive prerun response from being mistaken for a local
			// execution capability in the web app.
			if (backend.eventState.alwaysRemote === true) return "remote";

			const actionContext = action?.context ?? {};
			const actionAppId =
				action?.name === "workflow_event" &&
				typeof actionContext.appId === "string"
					? actionContext.appId
					: undefined;
			const actionBoardId =
				action?.name === "workflow_event" &&
				typeof actionContext.boardId === "string"
					? actionContext.boardId
					: undefined;
			const effectiveAppId = actionAppId || appId;
			const effectiveBoardId = actionBoardId || boardId;

			if (!effectiveAppId || !effectiveBoardId) return "remote";

			const effectiveVersion = resolveEventBoardVersion(
				boardId,
				// Version pins are meaningful only for a live page event. Builder and
				// preview surfaces always prerun the latest board.
				eventId ? boardVersion : undefined,
				effectiveBoardId,
			);
			if (!backend.boardState.prerunBoard) return "remote";

			try {
				const prerun = await prerunSwr(
					prerunBoardKey(effectiveAppId, effectiveBoardId, effectiveVersion),
					() =>
						backend.boardState.prerunBoard!(
							effectiveAppId,
							effectiveBoardId,
							effectiveVersion,
						),
				);

				return prerun.can_execute_locally &&
					prerun.execution_mode !== IExecutionMode.Remote
					? "local"
					: "remote";
			} catch (error) {
				console.warn(
					"[A2UI] Failed to resolve temporary upload execution target; using remote storage:",
					error,
				);
				return "remote";
			}
		},
		[
			appId,
			backend.boardState,
			backend.eventState.alwaysRemote,
			boardId,
			boardVersion,
			eventId,
		],
	);

	// Wrap onAction to intercept change events and store element values in memory
	const wrappedOnAction = useCallback(
		(message: A2UIClientMessage) => {
			// Store element values on change actions
			if (message.name === "change" && message.sourceComponentId) {
				const elementId = `${message.surfaceId}/${message.sourceComponentId}`;
				const context = message.context ?? {};
				const value = Object.prototype.hasOwnProperty.call(context, "value")
					? context.value
					: context.checked;

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
				boardVersion,
				eventId,
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
				setElementValue,
				resolveTemporaryUploadTarget,
				triggeringComponents,
				markComponentTriggering,
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
			boardVersion: undefined,
			eventId: undefined,
			surfaceId: "",
			isPreviewMode: false,
			resolveTemporaryUploadTarget: undefined,
			globalState: EMPTY_STATE,
			pageState: EMPTY_STATE,
		};
	}
	return {
		appId: context.appId,
		boardId: context.boardId,
		boardVersion: context.boardVersion,
		eventId: context.eventId,
		surfaceId: context.surfaceId,
		isPreviewMode: context.isPreviewMode,
		resolveTemporaryUploadTarget: context.resolveTemporaryUploadTarget,
		globalState: context.globalState,
		pageState: context.pageState,
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

/**
 * Hook returning the imperative element-value setter from ActionContext.
 * Micro widgets use it to publish their `value:changed` mirror under the
 * `"{instanceId}/values"` key so payload-based reads (Get Element Value)
 * stay coherent.
 */
export function useSetElementValue() {
	return useContext(ActionContext)?.setElementValue;
}

/**
 * Hook to access element values and components for building _input_values maps.
 * Used by WidgetActionHandler to collect event-relevant input values.
 */
export function useEventRelevantValues() {
	const context = useContext(ActionContext);
	const getElementValues = context?.getElementValues;
	const components = context?.components;
	const surfaceId = context?.surfaceId;

	const collectInputValues = useCallback((): Record<string, unknown> => {
		if (!getElementValues || !components || !surfaceId) return {};
		const storedValues = getElementValues();
		const inputValues: Record<string, unknown> = {};
		for (const [compId, comp] of Object.entries(components)) {
			if (!comp.eventRelevant) continue;
			const elementId = `${surfaceId}/${compId}`;
			if (storedValues[elementId] !== undefined) {
				inputValues[compId] = storedValues[elementId];
			}
		}
		return inputValues;
	}, [getElementValues, components, surfaceId]);

	return collectInputValues;
}

/**
 * Hook that returns whether a given component is currently triggering an action.
 * Used by interactive components to show a spinner while their action runs.
 */
export function useIsComponentTriggering(
	componentId: string | undefined,
): boolean {
	const context = useContext(ActionContext);
	if (!componentId) return false;
	return context?.triggeringComponents?.has(componentId) ?? false;
}

/**
 * Hook that returns the imperative `markComponentTriggering(id, on)` function
 * from ActionContext. Used by alternative action paths (e.g. WidgetActionHandler)
 * to register loading state in the same central map that A2UIButton consumes.
 */
export function useMarkComponentTriggering() {
	return useContext(ActionContext)?.markComponentTriggering;
}

/**
 * Hook to fetch and merge frontend elements for the current surface.
 * Returns a function that produces the `_elements` map sent in workflow payloads.
 */
export function useCollectEventElements() {
	const context = useContext(ActionContext);
	const backend = useBackend();
	const appId = context?.appId;
	const boardId = context?.boardId;
	const boardVersion = context?.boardVersion;
	const surfaceId = context?.surfaceId;
	const components = context?.components;
	const getElementValues = context?.getElementValues;

	return useCallback(async (): Promise<Record<string, unknown>> => {
		const fallbackElements = (): Record<string, unknown> =>
			flattenSurfaceComponentsForElements(components, surfaceId ?? "");

		let elementsMap: Record<string, unknown> | undefined;
		if (appId && boardId) {
			try {
				elementsMap = await backend.boardState.getExecutionElements(
					appId,
					boardId,
					surfaceId || "",
					false,
					boardVersion,
				);
				if (!elementsMap || Object.keys(elementsMap).length === 0) {
					elementsMap = fallbackElements();
				}
			} catch (err) {
				console.warn(
					"[A2UI] Failed to fetch execution elements, falling back to all components:",
					err,
				);
				elementsMap = fallbackElements();
			}
		} else {
			elementsMap = fallbackElements();
		}

		const storedValues = getElementValues?.() ?? {};
		return mergeStoredElementValues(
			elementsMap,
			storedValues,
			components,
			surfaceId || "",
		);
	}, [
		appId,
		boardId,
		boardVersion,
		surfaceId,
		components,
		getElementValues,
		backend.boardState,
	]);
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

export function useComponentActionTrigger(componentId: string | undefined) {
	const { executeAction } = useExecuteAction();

	return useCallback(
		(actions: Action[] | undefined, context: Record<string, unknown> = {}) => {
			const action = actions?.[0];
			if (!action) return Promise.resolve();
			return executeAction(action, componentId, context);
		},
		[componentId, executeAction],
	);
}

export interface ComponentEventSource {
	actions?: Action[];
	eventHandlers?: EventHandlers;
}

export interface ComponentEventTriggerOptions {
	/** Some newly exposed events had no legacy configured-action behavior. */
	legacyFallback?: boolean;
	/** Events added after a component shipped do not inherit its `*` handler. */
	wildcardFallback?: boolean;
}

/**
 * Execute the ordered actions configured for a semantic component event.
 * Exact/wildcard handlers override the legacy singleton `actions[0]` fallback.
 */
export function useComponentEventTrigger(componentId: string | undefined) {
	const { executeAction } = useExecuteAction();

	return useCallback(
		async (
			eventName: string,
			component: ComponentEventSource,
			context: Record<string, unknown> = {},
			options: ComponentEventTriggerOptions = {},
		) => {
			const resolution = resolveEventActions(
				component.eventHandlers,
				eventName,
				component.actions,
				options,
			);
			for (const action of resolution.actions) {
				await executeAction(action, componentId, context);
			}
		},
		[componentId, executeAction],
	);
}

export function useExecuteAction() {
	const router = useRouter();
	const pathname = usePathname();
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const widgetInstance = useWidgetInstance();
	const executeActionRef = useRef<ExecuteActionFn | null>(null);
	const {
		onAction,
		onA2UIMessage,
		surfaceId,
		appId,
		boardId,
		boardVersion,
		eventId,
		components,
		globalState,
		pageState,
		isPreviewMode,
		openDialog,
		closeDialog,
		getElementValues,
		markComponentTriggering,
	} = useContext(ActionContext) ?? {};

	const handleA2UIEvents = useCallback(
		(events: IIntercomEvent[]) => {
			console.log("[A2UI] Received events from backend:", events);

			for (const event of events) {
				console.log(
					"[A2UI] Processing event:",
					event.event_type,
					event.payload,
				);

				if (event.event_type === "error") {
					const payload = event.payload as { message?: unknown } | undefined;
					const message =
						typeof payload?.message === "string"
							? payload.message
							: "The workflow could not be completed.";
					toast.error("Workflow execution failed", { description: message });
					continue;
				}

				if (event.event_type === "a2ui") {
					const message = event.payload as A2UIServerMessage;
					console.log("[A2UI] A2UI message:", message);

					if (handleWidgetQueryMessage(message)) {
						continue;
					}

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

						if (replace) {
							router.replace(navUrl);
						} else {
							router.push(navUrl);
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
		[router, pathname, onA2UIMessage, appId, openDialog, closeDialog],
	);

	const executeAction = useCallback(
		async (
			action: Action | undefined,
			triggeringComponentId?: string,
			additionalContext: Record<string, unknown> = {},
		) => {
			// Only execute actions in preview mode
			if (!isPreviewMode || !action) return;

			const { name } = action;
			const context = { ...(action.context ?? {}), ...additionalContext };

			console.log("[ActionHandler] executeAction:", {
				name,
				context,
				appId,
				isPreviewMode,
				triggeringComponentId,
			});

			if (triggeringComponentId) {
				markComponentTriggering?.(triggeringComponentId, true);
			}

			try {
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
					case "navigate_app_config": {
						const contextAppId = context.appId as string | undefined;
						const targetAppId = contextAppId || appId;
						if (!targetAppId) {
							console.warn("[A2UI] navigate_app_config missing appId");
							break;
						}

						router.push(
							`/library/config?id=${encodeURIComponent(targetAppId)}`,
						);
						break;
					}
					case "navigate_app_overview": {
						const contextAppId = context.appId as string | undefined;
						const contextEventId = context.eventId as string | undefined;
						const targetAppId = contextAppId || appId;
						const targetEventId = contextEventId || eventId;
						if (!targetAppId) {
							console.warn("[A2UI] navigate_app_overview missing appId");
							break;
						}

						const params = new URLSearchParams();
						params.set("id", targetAppId);
						if (targetEventId) params.set("eventId", targetEventId);
						router.push(`/store?${params.toString()}`);
						break;
					}
					case "submit_feedback": {
						const contextAppId = context.appId as string | undefined;
						const contextEventId = context.eventId as string | undefined;
						const targetAppId = contextAppId || appId;
						const targetEventId = contextEventId || eventId;

						if (!targetAppId || !targetEventId) {
							console.warn("[A2UI] submit_feedback missing appId or eventId", {
								appId: targetAppId,
								eventId: targetEventId,
							});
							toast.error("Feedback is not available on this page.");
							break;
						}

						const rawRating = Number(context.rating ?? 5);
						const rating = Number.isFinite(rawRating)
							? Math.max(0, Math.min(5, Math.round(rawRating)))
							: 5;
						const feedbackId =
							typeof context.feedbackId === "string" &&
							context.feedbackId.trim()
								? context.feedbackId.trim()
								: (triggeringComponentId ?? "feedback");
						const namespacedFeedbackId = `${surfaceId}:${feedbackId}`;

						let comment =
							typeof context.comment === "string" ? context.comment : "";
						const commentComponentId = context.commentComponentId as
							| string
							| undefined;
						const storedElementValues = getElementValues?.() ?? {};
						if (commentComponentId) {
							const elementId = commentComponentId.includes("/")
								? commentComponentId
								: `${surfaceId}/${commentComponentId}`;
							const value = storedElementValues[elementId];
							if (value !== undefined && value !== null) {
								comment = String(value);
							}
						}

						const includeState = context.includeState !== false;
						const pageContext = getCurrentPageContext(pathname, {
							mode:
								typeof context.pageContextMode === "string"
									? context.pageContextMode
									: "path",
							queryParamAllowlist:
								typeof context.pageContextQueryParamAllowlist === "string"
									? context.pageContextQueryParamAllowlist
									: undefined,
							queryParamDenylist:
								typeof context.pageContextQueryParamDenylist === "string"
									? context.pageContextQueryParamDenylist
									: undefined,
							includeHash: context.includePageHash === true,
						});
						const localState = {
							...(includeState
								? {
										...pageState,
										elementValues: storedElementValues,
									}
								: {}),
							pageContext,
						};

						await backend.eventState.upsertEventFeedback(
							targetAppId,
							targetEventId,
							namespacedFeedbackId,
							{
								rating,
								comment,
								globalState: includeState ? globalState : undefined,
								localState,
							},
						);

						const successMessage =
							typeof context.successMessage === "string"
								? context.successMessage
								: "Thanks for the feedback.";
						toast.success(successMessage);
						break;
					}
					case "workflow_event": {
						const nodeId = context.nodeId as string | undefined;
						const actionBoardId = context.boardId as string | undefined;
						const contextAppId = context.appId as string | undefined;

						const effectiveAppId = contextAppId || appId;
						const effectiveBoardId = actionBoardId || boardId;
						const inheritedBoardVersion = resolveEventBoardVersion(
							boardId,
							boardVersion,
							effectiveBoardId,
						);

						if (nodeId && effectiveBoardId && effectiveAppId) {
							try {
								const cacheKey = `${effectiveBoardId}:${surfaceId}`;
								let elementsMap: Record<string, unknown> | undefined;

								console.log("[A2UI] workflow_event execution context:", {
									nodeId,
									effectiveAppId,
									effectiveBoardId,
									surfaceId,
									cacheKey,
									hadCachedElements: false,
									componentIds: Object.keys(components ?? {}),
								});

								try {
									const currentBoard = await backend.boardState.getBoard(
										effectiveAppId,
										effectiveBoardId,
										inheritedBoardVersion,
									);
									console.log(
										"[A2UI] workflow_event local board element refs:",
										{
											boardId: effectiveBoardId,
											pageIds: currentBoard.page_ids,
											updatedAt: currentBoard.updated_at,
											elementRefs: summarizeBoardElementRefs(currentBoard),
										},
									);
								} catch (boardErr) {
									console.warn(
										"[A2UI] Failed to fetch current board for workflow_event diagnostics:",
										boardErr,
									);
								}

								// Always fetch current execution elements in preview mode.
								// The flow graph can change without any cache invalidation signal.
								try {
									elementsMap = await backend.boardState.getExecutionElements(
										effectiveAppId,
										effectiveBoardId,
										surfaceId || "",
										false,
										inheritedBoardVersion,
									);

									if (!elementsMap || Object.keys(elementsMap).length === 0) {
										elementsMap = flattenSurfaceComponentsForElements(
											components,
											surfaceId ?? "",
										);
									}
								} catch (err) {
									console.warn(
										"[A2UI] Failed to fetch execution elements, falling back to all components:",
										err,
									);
									elementsMap = flattenSurfaceComponentsForElements(
										components,
										surfaceId ?? "",
									);
								}

								// Merge in-memory element values (user input state)
								const storedValues = getElementValues?.() ?? {};
								console.log("[A2UI] workflow_event merging elements:", {
									elementsMapKeys: Object.keys(elementsMap),
									storedValuesKeys: Object.keys(storedValues),
									storedValues,
								});

								for (const [elementId, element] of Object.entries(
									elementsMap,
								)) {
									const storedValue = storedValues[elementId];
									console.log("[A2UI] Checking element:", {
										elementId,
										hasStoredValue: storedValue !== undefined,
										storedValue,
									});
								}

								const mergedElements = mergeStoredElementValues(
									elementsMap,
									storedValues,
									components,
									surfaceId || "",
								);

								// Build _input_values from event-relevant components
								const inputValues: Record<string, unknown> = {};
								if (components && surfaceId) {
									for (const [compId, comp] of Object.entries(components)) {
										if (!comp.eventRelevant) continue;
										const elementId = `${surfaceId}/${compId}`;
										if (storedValues[elementId] !== undefined) {
											inputValues[compId] = storedValues[elementId];
										}
									}
								}

								const basePayload = compactWorkflowPayload({
									id: nodeId,
									payload: {
										_elements: mergedElements,
										_input_values: inputValues,
										_action_context: context,
										_triggering_component_id: triggeringComponentId ?? "",
										...buildFrontendContextPayload(
											pathname,
											globalState,
											pageState,
										),
									},
								}) as {
									id: string;
									payload: Record<string, unknown>;
								};
								const payload = withBoardVersion(
									basePayload,
									inheritedBoardVersion,
								);

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
								toast.error("Workflow execution failed", {
									description:
										error instanceof Error
											? error.message
											: "The workflow could not be started.",
								});
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
						console.log("[A2UI] widget_event triggered:", {
							context,
							widgetInstance,
							appId,
							boardId,
						});
						const actionId = context.actionId as string | undefined;
						if (!actionId) {
							console.warn("[A2UI] widget_event missing actionId");
							toast.warning("Widget action has no action id configured.");
							break;
						}

						const route = resolveWidgetInstanceEventRoute(
							widgetInstance,
							actionId,
						);
						if (route.kind === "actions") {
							const executeNestedAction = executeActionRef.current;
							if (executeNestedAction) {
								for (const routedAction of route.actions) {
									await executeNestedAction(
										routedAction,
										widgetInstance?.componentId ?? triggeringComponentId,
										context,
									);
								}
							}
							break;
						}

						if (route.kind === "diagnostic") {
							const available = Object.keys(
								widgetInstance?.actionBindings ?? {},
							);
							console.warn(
								"[A2UI] widget_event: no binding found for actionId:",
								actionId,
								"available bindings:",
								widgetInstance?.actionBindings,
							);
							toast.warning(
								`Widget action '${actionId}' is not bound to a workflow${
									available.length
										? ` (bound: ${available.join(", ")})`
										: ". Reference a Widget Action Event from the Instantiate Widget node, then re-run the flow so a fresh widget is pushed."
								}`,
							);
							break;
						}

						// Keep the classic binding execution path unchanged.
						const binding = route.binding;

						if (!("workflow" in binding)) {
							console.warn(
								"[A2UI] widget_event: only workflow bindings are supported for execution, got:",
								binding,
							);
							toast.warning(
								`Widget action '${actionId}' has a non-workflow binding and cannot run here.`,
							);
							break;
						}

						const nodeId = binding.workflow.flowId;

						const effectiveAppId = appId;
						const effectiveBoardId = boardId;
						const inheritedBoardVersion = resolveEventBoardVersion(
							boardId,
							boardVersion,
							effectiveBoardId,
						);

						if (effectiveBoardId && effectiveAppId) {
							try {
								const cacheKey = `${effectiveBoardId}:${surfaceId}`;
								let elementsMap: Record<string, unknown> | undefined;

								console.log("[A2UI] widget_event execution context:", {
									effectiveAppId,
									effectiveBoardId,
									surfaceId,
									cacheKey,
									hadCachedElements: false,
									componentIds: Object.keys(components ?? {}),
								});

								try {
									elementsMap = await backend.boardState.getExecutionElements(
										effectiveAppId,
										effectiveBoardId,
										surfaceId || "",
										false,
										inheritedBoardVersion,
									);

									if (!elementsMap || Object.keys(elementsMap).length === 0) {
										elementsMap = flattenSurfaceComponentsForElements(
											components,
											surfaceId ?? "",
										);
									}
								} catch (err) {
									console.warn(
										"[A2UI] Failed to fetch execution elements for widget_event:",
										err,
									);
									elementsMap = flattenSurfaceComponentsForElements(
										components,
										surfaceId ?? "",
									);
								}

								const storedValues = getElementValues?.() ?? {};
								const mergedElements = mergeStoredElementValues(
									elementsMap,
									storedValues,
									components,
									surfaceId || "",
								);

								// Build _input_values from event-relevant components
								const inputValues: Record<string, unknown> = {};
								if (components && surfaceId) {
									for (const [compId, comp] of Object.entries(components)) {
										if (!comp.eventRelevant) continue;
										const elementId = `${surfaceId}/${compId}`;
										if (storedValues[elementId] !== undefined) {
											inputValues[compId] = storedValues[elementId];
										}
									}
								}

								const basePayload = compactWorkflowPayload({
									id: nodeId,
									payload: {
										_elements: mergedElements,
										_input_values: inputValues,
										_widget_instance_id: widgetInstance?.instanceId ?? "",
										_action_id: actionId,
										_action_context: context,
										_triggering_component_id: triggeringComponentId ?? "",
										...buildFrontendContextPayload(
											pathname,
											globalState,
											pageState,
										),
									},
								}) as { id: string; payload: Record<string, unknown> };
								const payload = withBoardVersion(
									basePayload,
									inheritedBoardVersion,
								);

								const execFn =
									executionService?.executeBoard ??
									backend.boardState.executeBoard;
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
								toast.error(`Widget action '${actionId}' failed`, {
									description:
										error instanceof Error
											? error.message
											: "The widget workflow could not be started.",
								});
							}
						} else {
							console.warn(
								"[A2UI] Missing appId or boardId for widget_event:",
								{
									appId: effectiveAppId,
									boardId: effectiveBoardId,
								},
							);
							toast.warning(
								"Widget action cannot run: missing app or board context.",
							);
						}
						break;
					}
					default:
						if (onAction) {
							onAction({
								type: "userAction",
								name,
								surfaceId: surfaceId ?? "",
								sourceComponentId: triggeringComponentId ?? "",
								timestamp: Date.now(),
								context,
							});
						}
				}
			} finally {
				if (triggeringComponentId) {
					markComponentTriggering?.(triggeringComponentId, false);
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
			boardVersion,
			eventId,
			components,
			globalState,
			pageState,
			handleA2UIEvents,
			isPreviewMode,
			widgetInstance,
			getElementValues,
			markComponentTriggering,
		],
	);
	executeActionRef.current = executeAction;

	return { executeAction, isPreviewMode: isPreviewMode ?? false };
}
