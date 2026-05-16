"use client";

import { useCallback, useId, useMemo } from "react";
import { createSanitizedStyleProps, safeScopedCss } from "../../lib/css-utils";
import { cn } from "../../lib/utils";
import { ActionProvider } from "./ActionHandler";
import { type ComponentProps, getComponentRenderer } from "./ComponentRegistry";
import { DataProvider, DataScopeProvider } from "./DataContext";
import { type IWidgetRef, WidgetRefsProvider } from "./WidgetRefsContext";
import type {
	A2UIClientMessage,
	A2UIServerMessage,
	DataEntry,
	DataScope,
	Surface,
	SurfaceComponent,
} from "./types";

const EMPTY_DATA_MODEL: DataEntry[] = [];

function isBackgroundClass(value: string | undefined): value is string {
	return value?.startsWith("bg-") ?? false;
}

export interface A2UIRendererProps {
	surface: Surface;
	widgetRefs?: Record<string, IWidgetRef>;
	onMessage?: (message: A2UIClientMessage) => void;
	onA2UIMessage?: (message: A2UIServerMessage) => void;
	className?: string;
	appId?: string;
	boardId?: string;
	isPreviewMode?: boolean;
	openDialog?: (
		route: string,
		title?: string,
		queryParams?: Record<string, string>,
		dialogId?: string,
	) => void;
	closeDialog?: (dialogId?: string) => void;
}

export function A2UIRenderer({
	surface,
	widgetRefs,
	onMessage,
	onA2UIMessage,
	className,
	appId,
	boardId,
	isPreviewMode = false,
	openDialog,
	closeDialog,
}: A2UIRendererProps) {
	const canvasId = useId();
	const components = useMemo(
		() => surface.components ?? {},
		[surface.components],
	);
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

	const renderScopedComponent = useCallback(
		(componentId: string, dataScope?: DataScope): React.ReactNode => {
			const surfaceComponent = components[componentId];
			if (!surfaceComponent?.component) return null;

			const { component, style } = surfaceComponent;
			const Renderer = getComponentRenderer(component.type);
			if (!Renderer) {
				console.warn(`Unknown component type: ${component.type}`);
				return null;
			}

			const props: ComponentProps = {
				component,
				componentId,
				surfaceId: surface.id,
				appId,
				boardId,
				style: style ?? component.style,
				onAction: handleAction,
				renderChild: (childId, childScope) =>
					renderScopedComponent(childId, childScope ?? dataScope),
			};

			const node = <Renderer key={componentId} {...props} />;
			return dataScope ? (
				<DataScopeProvider scope={dataScope}>{node}</DataScopeProvider>
			) : (
				node
			);
		},
		[components, surface.id, appId, boardId, handleAction],
	);

	const rootComponent = surface.rootComponentId
		? components[surface.rootComponentId]
		: null;

	if (!rootComponent) {
		return (
			<div className={className}>
				<div className="text-muted-foreground text-sm">
					No content to display
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
					components={components}
					isPreviewMode={isPreviewMode}
					openDialog={openDialog}
					closeDialog={closeDialog}
				>
					{customCss && (
						<style
							{...createSanitizedStyleProps(
								safeScopedCss(
									customCss,
									`[data-surface-canvas-id="${canvasId}"]`,
								),
							)}
						/>
					)}
					<div
						className={cn(backgroundClass, className)}
						data-surface-canvas-id={canvasId}
						style={canvasStyle}
					>
						{renderScopedComponent(surface.rootComponentId)}
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
