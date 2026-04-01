"use client";

import { createContext, useCallback, useContext } from "react";
import {
	type ComponentProps,
	getComponentRenderer,
} from "../ComponentRegistry";
import { useWidgetRefs } from "../WidgetRefsContext";
import type { A2UIComponent, ActionBinding, Style } from "../types";

export interface InlineWidgetDef {
	name: string;
	rootComponentId: string;
	components: { id: string; component: Record<string, unknown>; style?: Style }[];
}

export interface WidgetInstanceComponentProps {
	widgetId: string;
	instanceId: string;
	appId?: string;
	exposedPropValues?: Record<string, unknown>;
	styleOverride?: Record<string, unknown>;
	actionBindings?: Record<string, ActionBinding>;
	style?: Style;
	inlineWidgetDef?: InlineWidgetDef;
}

interface WidgetInstanceContextValue {
	instanceId: string;
	widgetId: string;
	actionBindings: Record<string, ActionBinding>;
}

const WidgetInstanceContext = createContext<WidgetInstanceContextValue | null>(null);

export function useWidgetInstance(): WidgetInstanceContextValue | null {
	return useContext(WidgetInstanceContext);
}

/**
 * A2UIWidgetInstance renders a widget instance by looking up the widget definition
 * from widgetRefs (stored on the page) and rendering its component tree.
 */
export function A2UIWidgetInstance({
	component,
	componentId,
	surfaceId,
	onAction,
}: ComponentProps) {
	const props = component as unknown as WidgetInstanceComponentProps;
	const { instanceId, widgetId, inlineWidgetDef, actionBindings } = props;
	const widgetRefsContext = useWidgetRefs();

	// Get widget definition from refs, fall back to inline definition
	const fromRefs = widgetRefsContext?.getWidgetRef(instanceId);
	const widgetDef = fromRefs ?? inlineWidgetDef;

	// Create a local renderChild for the widget's internal components
	const renderWidgetChild = useCallback(
		(childId: string, currentWidgetDef: typeof widgetDef): React.ReactNode => {
			if (!currentWidgetDef) return null;

			const childComponent = currentWidgetDef.components.find(
				(c) => c.id === childId,
			);
			if (!childComponent?.component) {
				console.warn(
					`Widget "${currentWidgetDef.name}" component "${childId}" not found. Available components:`,
					currentWidgetDef.components.map((c) => c.id),
				);
				return null;
			}

			const componentType = childComponent.component.type as string;
			const Renderer = getComponentRenderer(componentType);
			if (!Renderer) {
				console.warn(
					`Unknown component type: ${componentType}`,
				);
				return null;
			}

			return (
				<Renderer
					key={childId}
					component={childComponent.component as A2UIComponent}
					componentId={childId}
					surfaceId={surfaceId}
					style={childComponent.style ?? (childComponent.component.style as Style | undefined)}
					onAction={onAction}
					renderChild={(nestedChildId) =>
						renderWidgetChild(nestedChildId, currentWidgetDef)
					}
				/>
			);
		},
		[surfaceId, onAction],
	);

	if (!widgetDef) {
		return (
			<div className="p-4 text-sm text-red-500 bg-red-50 rounded">
				Widget instance &quot;{instanceId}&quot; not found in refs
			</div>
		);
	}

	if (!widgetDef.rootComponentId) {
		return (
			<div className="p-4 text-sm text-red-500 bg-red-50 rounded">
				Widget definition missing rootComponentId
		</div>
		);
	}

	console.log("[A2UI WidgetInstance] rendering:", { instanceId, widgetId, actionBindings, hasInlineDef: !!inlineWidgetDef, rootComponentId: widgetDef?.rootComponentId });

	return (
		<WidgetInstanceContext.Provider
			value={{
				instanceId,
				widgetId,
				actionBindings: actionBindings ?? {},
			}}
		>
			<div
				data-widget-instance={instanceId}
				data-widget-id={widgetId}
				className="contents"
			>
				{renderWidgetChild(widgetDef.rootComponentId, widgetDef)}
			</div>
		</WidgetInstanceContext.Provider>
	);
}
