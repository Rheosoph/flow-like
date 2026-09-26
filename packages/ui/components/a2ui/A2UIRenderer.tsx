"use client";

import { useTranslation } from "@flow-like/locales";
import {
	createContext,
	memo,
	useCallback,
	useContext,
	useId,
	useLayoutEffect,
	useMemo,
	useRef,
} from "react";
import type { BoardVersion } from "../../lib/schema/flow/board-version";
import { useRuntimeTailwindStyles } from "../../lib/use-runtime-tailwind";
import { cn } from "../../lib/utils";
import { ScopedCustomCss } from "../scoped-custom-css";
import { ActionProvider } from "./ActionHandler";
import {
	type ComponentProps,
	type RenderChildFn,
	getComponentRenderer,
} from "./ComponentRegistry";
import { DataProvider, DataScopeProvider, useData } from "./DataContext";
import { type IWidgetRef, WidgetRefsProvider } from "./WidgetRefsContext";
import type { RunElementDemand } from "./collect-run-elements";
import {
	type ComponentStore,
	createComponentStore,
	useSurfaceComponent,
} from "./component-store";
import type { A2UINavigationMessageInterceptor } from "./navigation-message";
import {
	PENDING_PAGE_ACTION_ATTRIBUTE,
	hasPendingPageAction,
} from "./pending-page-action";
import { resolveHidden } from "./resolve-hidden";
import type {
	A2UIClientMessage,
	A2UIServerMessage,
	BoundValue,
	DataEntry,
	DataScope,
	Style,
	Surface,
	SurfaceComponent,
} from "./types";

const EMPTY_DATA_MODEL: DataEntry[] = [];

function isBackgroundClass(value: string | undefined): value is string {
	return value?.startsWith("bg-") ?? false;
}

function resolveStyleBindings(
	style: Style | undefined,
	resolve: (boundValue: BoundValue, defaultValue?: unknown) => unknown,
): Style | undefined {
	if (!style?.background || !("image" in style.background)) return style;

	const resolvedUrl = resolve(style.background.image.url, "");
	return {
		...style,
		background: {
			image: {
				...style.background.image,
				url: {
					literalString:
						typeof resolvedUrl === "string"
							? resolvedUrl
							: String(resolvedUrl ?? ""),
				},
			},
		},
	};
}

interface SurfaceRenderContextValue {
	store: ComponentStore;
	surfaceId: string;
	appId?: string;
	boardId?: string;
	handleAction: (message: A2UIClientMessage) => void;
}

const SurfaceRenderContext = createContext<SurfaceRenderContextValue | null>(
	null,
);

function useSurfaceRender(): SurfaceRenderContextValue {
	const context = useContext(SurfaceRenderContext);
	if (!context) {
		throw new Error("A2UIComponentNode rendered outside an A2UIRenderer");
	}
	return context;
}

interface A2UIComponentNodeProps {
	componentId: string;
	/** Scope the parent rendered this node in; children rendered without one inherit it. */
	inheritedScope?: DataScope;
}

/**
 * Element for one surface component. Everything a node needs comes from
 * context or the store, so the element's props never change for an unchanged
 * component — a parent can re-render without touching it.
 */
function renderScopedComponent(
	componentId: string,
	dataScope?: DataScope,
): React.ReactNode {
	const node = (
		<A2UIComponentNode
			key={componentId}
			componentId={componentId}
			inheritedScope={dataScope}
		/>
	);
	return dataScope ? (
		<DataScopeProvider key={componentId} scope={dataScope}>
			{node}
		</DataScopeProvider>
	) : (
		node
	);
}

/**
 * Memoized and subscribed to its own component only: a surface update
 * re-renders the nodes whose entries changed, not the whole tree. A node still
 * re-renders when a context it reads changes (data model, actions, widget
 * refs), which is exactly when its output can differ.
 */
const A2UIComponentNode = memo(function A2UIComponentNode({
	componentId,
	inheritedScope,
}: A2UIComponentNodeProps) {
	const { store, surfaceId, appId, boardId, handleAction } = useSurfaceRender();
	const surfaceComponent = useSurfaceComponent(store, componentId);
	const { resolve } = useData();
	const actionPending = hasPendingPageAction(surfaceComponent?.component);
	const elementRef = useCallback(
		(element: HTMLElement | SVGElement | null) => {
			element?.setAttribute(
				"data-a2ui-element-ref",
				`${surfaceId}/${componentId}`,
			);
			element?.toggleAttribute(PENDING_PAGE_ACTION_ATTRIBUTE, actionPending);
		},
		[surfaceId, componentId, actionPending],
	);
	const renderChild = useCallback<RenderChildFn>(
		(childId, childScope) =>
			renderScopedComponent(childId, childScope ?? inheritedScope),
		[inheritedScope],
	);

	const component = surfaceComponent?.component;
	if (!component || resolveHidden(component.hidden, resolve)) return null;
	const resolvedStyle = resolveStyleBindings(
		surfaceComponent.style ?? component.style,
		resolve,
	);

	const Renderer = getComponentRenderer(component.type);
	if (!Renderer) {
		console.warn(`Unknown component type: ${component.type}`);
		return null;
	}

	const props: ComponentProps = {
		component,
		componentId,
		surfaceId,
		appId,
		boardId,
		style: resolvedStyle,
		elementRef,
		onAction: handleAction,
		renderChild,
	};

	return <Renderer {...props} />;
});

export interface A2UIRendererProps {
	surface: Surface;
	widgetRefs?: Record<string, IWidgetRef>;
	onMessage?: (message: A2UIClientMessage) => void;
	onA2UIMessage?: (message: A2UIServerMessage) => void;
	className?: string;
	appId?: string;
	boardId?: string;
	boardVersion?: BoardVersion;
	eventId?: string;
	/** True when workflow routes must come from the Page Event projection. */
	governedPage?: boolean;
	/** Page elements the Page Event's board reads, from its bootstrap. */
	elementDemand?: RunElementDemand;
	isPreviewMode?: boolean;
	openDialog?: (
		route: string,
		title?: string,
		queryParams?: Record<string, string>,
		dialogId?: string,
	) => void;
	closeDialog?: (dialogId?: string) => void;
	/** Consume page navigation inside an embedded owner instead of changing the host router. */
	onNavigationMessage?: A2UINavigationMessageInterceptor;
	/** Mounted inside the ActionProvider; used by pages to register the FlowPilot live-page handle. */
	agentBridge?: React.ReactNode;
}

export function A2UIRenderer({
	surface,
	widgetRefs,
	onMessage,
	onA2UIMessage,
	className,
	appId,
	boardId,
	boardVersion,
	eventId,
	governedPage = false,
	elementDemand,
	isPreviewMode = false,
	openDialog,
	closeDialog,
	onNavigationMessage,
	agentBridge,
}: A2UIRendererProps) {
	const { t } = useTranslation("common");
	const canvasId = useId();
	const canvasRef = useRef<HTMLDivElement>(null);
	useRuntimeTailwindStyles(canvasRef);
	const components = useMemo(
		() => surface.components ?? {},
		[surface.components],
	);
	// Nodes read the record during this render; the changed ids are notified
	// once the tree has committed (see component-store.ts).
	const storeRef = useRef<ComponentStore | null>(null);
	if (!storeRef.current) storeRef.current = createComponentStore(components);
	const store = storeRef.current;
	store.replace(components);
	useLayoutEffect(() => {
		store.commit(components);
	}, [store, components]);
	const canvasSettings = surface.canvasSettings;
	const dataModel = surface.dataModel ?? EMPTY_DATA_MODEL;
	const backgroundClass = isBackgroundClass(canvasSettings?.backgroundColor)
		? canvasSettings?.backgroundColor
		: undefined;
	const canvasStyle = useMemo(
		() => ({
			backgroundColor: backgroundClass
				? undefined
				: canvasSettings?.backgroundColor,
			backgroundImage: canvasSettings?.backgroundImage
				? `url(${canvasSettings.backgroundImage})`
				: undefined,
			backgroundSize: canvasSettings?.backgroundImage ? "cover" : undefined,
			backgroundPosition: canvasSettings?.backgroundImage
				? "center"
				: undefined,
			padding: canvasSettings?.padding,
		}),
		[canvasSettings, backgroundClass],
	);
	const customCss = canvasSettings?.customCss;

	const handleAction = useCallback(
		(message: A2UIClientMessage) => {
			onMessage?.(message);
		},
		[onMessage],
	);

	const renderContext = useMemo<SurfaceRenderContextValue>(
		() => ({ store, surfaceId: surface.id, appId, boardId, handleAction }),
		[store, surface.id, appId, boardId, handleAction],
	);

	const rootComponent = surface.rootComponentId
		? components[surface.rootComponentId]
		: null;

	if (!rootComponent) {
		return (
			<div ref={canvasRef} className={className}>
				<div className="text-muted-foreground text-sm">
					{t("noContentToDisplay", "No content to display")}
				</div>
			</div>
		);
	}

	return (
		<DataProvider initialData={dataModel}>
			<WidgetRefsProvider widgetRefs={widgetRefs}>
				<ActionProvider
					onAction={handleAction}
					onA2UIMessage={onA2UIMessage}
					surfaceId={surface.id}
					appId={appId}
					boardId={boardId}
					boardVersion={boardVersion}
					eventId={eventId}
					governedPage={governedPage}
					elementDemand={elementDemand}
					components={components}
					isPreviewMode={isPreviewMode}
					openDialog={openDialog}
					closeDialog={closeDialog}
					onNavigationMessage={onNavigationMessage}
				>
					{agentBridge}
					<ScopedCustomCss
						css={customCss}
						scopeSelector={`[data-surface-canvas-id="${canvasId}"]`}
					/>
					<div
						ref={canvasRef}
						className={cn(
							"**:data-a2ui-action-pending:pointer-events-none **:data-a2ui-action-pending:opacity-50",
							backgroundClass,
							className,
						)}
						data-surface-canvas-id={canvasId}
						style={canvasStyle}
					>
						<SurfaceRenderContext.Provider value={renderContext}>
							{renderScopedComponent(surface.rootComponentId)}
						</SurfaceRenderContext.Provider>
					</div>
				</ActionProvider>
			</WidgetRefsProvider>
		</DataProvider>
	);
}

export interface A2UIMessageHandlerProps {
	onServerMessage: (message: A2UIServerMessage) => void;
	children: (props: {
		surfaces: Map<string, Surface>;
		sendMessage: (msg: A2UIClientMessage) => void;
	}) => React.ReactNode;
}

export function useA2UIState() {
	const applyServerMessage = useCallback(
		(
			surfaces: Map<string, Surface>,
			message: A2UIServerMessage,
		): Map<string, Surface> => {
			const newSurfaces = new Map(surfaces);

			if (message.type === "beginRendering") {
				const componentsMap: Record<string, SurfaceComponent> = {};
				for (const comp of message.components) {
					componentsMap[comp.id] = comp;
				}
				newSurfaces.set(message.surfaceId, {
					id: message.surfaceId,
					rootComponentId: message.rootComponentId,
					components: componentsMap,
					dataModel: message.dataModel,
					catalogId: message.catalogId,
				});
			}

			if (message.type === "surfaceUpdate") {
				const existing = newSurfaces.get(message.surfaceId);
				if (existing) {
					const updatedComponents = { ...existing.components };
					for (const comp of message.components) {
						updatedComponents[comp.id] = comp;
					}
					newSurfaces.set(message.surfaceId, {
						...existing,
						components: updatedComponents,
					});
				}
			}

			if (message.type === "setCanvasSettings") {
				const existing = newSurfaces.get(message.surfaceId);
				if (existing) {
					newSurfaces.set(message.surfaceId, {
						...existing,
						canvasSettings: {
							...existing.canvasSettings,
							...message.canvasSettings,
						},
					});
				}
			}

			if (message.type === "dataModelUpdate") {
				const existing = newSurfaces.get(message.surfaceId);
				if (existing) {
					const entries = new Map(
						(existing.dataModel ?? []).map((entry) => [entry.path, entry]),
					);
					for (const entry of message.contents) {
						entries.set(entry.path, entry);
					}
					newSurfaces.set(message.surfaceId, {
						...existing,
						dataModel: Array.from(entries.values()),
					});
				}
			}

			if (message.type === "deleteSurface") {
				newSurfaces.delete(message.surfaceId);
			}

			return newSurfaces;
		},
		[],
	);

	return { applyServerMessage };
}
