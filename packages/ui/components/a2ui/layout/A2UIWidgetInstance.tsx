"use client";

import { useTranslation } from "@flow-like/locales";
import { createContext, useCallback, useContext, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	type ComponentProps,
	getComponentRenderer,
} from "../ComponentRegistry";
import { useData } from "../DataContext";
import { useWidgetRefs } from "../WidgetRefsContext";
import { applyElementUpdate } from "../apply-a2ui-message";
import { resolveEventActions } from "../event-handlers";
import { resolveHidden } from "../resolve-hidden";
import type {
	A2UIComponent,
	Action,
	ActionBinding,
	EventHandlers,
	Style,
} from "../types";

export interface InlineWidgetDef {
	name: string;
	rootComponentId: string;
	components: {
		id: string;
		component: Record<string, unknown>;
		style?: Style;
		eventRelevant?: boolean;
	}[];
}

/** Minimal shape of a widget's parameter declaration (mirrors ExposedProp in widget-state). */
export interface ExposedPropDef {
	id: string;
	targetComponentId: string;
	/** Dot path on the target component, e.g. "content", "style.className", "data.rows". */
	propertyPath: string;
	propType?: unknown;
}

export type WidgetComponentDef = {
	id: string;
	component?: Record<string, unknown>;
	style?: unknown;
	eventRelevant?: boolean;
};

function isBoundValue(value: unknown): boolean {
	return (
		typeof value === "object" &&
		value !== null &&
		("literalString" in value ||
			"literalNumber" in value ||
			"literalBool" in value ||
			"literalOptions" in value ||
			"path" in value)
	);
}

/**
 * Shape an instance's raw parameter value for the target property. Content-style props are consumed
 * as BoundValues (`{literalString}`/…), while className/style/json props are applied verbatim.
 */
function shapeExposedValue(value: unknown, propType: unknown): unknown {
	if (isBoundValue(value)) return value;
	if (propType === "Number") return { literalNumber: Number(value) };
	if (propType === "Boolean") return { literalBool: Boolean(value) };
	if (
		propType === "TailwindClass" ||
		propType === "StyleObject" ||
		propType === "Json"
	) {
		return value;
	}
	return { literalString: value == null ? "" : String(value) };
}

function setDeep(
	target: Record<string, unknown>,
	parts: string[],
	value: unknown,
): void {
	let cursor = target;
	for (let i = 0; i < parts.length - 1; i++) {
		const key = parts[i];
		if (typeof cursor[key] !== "object" || cursor[key] === null) {
			cursor[key] = {};
		}
		cursor = cursor[key] as Record<string, unknown>;
	}
	cursor[parts[parts.length - 1]] = value;
}

/**
 * Apply a widget instance's `exposedPropValues` onto a copy of the widget's components. Each
 * declared prop maps its value onto `targetComponentId` at `propertyPath` — `style.*` paths target
 * the component's style, everything else targets its props. Returns the original array untouched
 * when there is nothing to override (the common case), so unchanged widgets keep referential
 * identity and don't re-render.
 */
export function applyExposedProps<T extends WidgetComponentDef>(
	components: T[],
	exposedProps: ExposedPropDef[],
	values: Record<string, unknown> | undefined,
): T[] {
	if (!values || exposedProps.length === 0) return components;
	const overrides = exposedProps
		.map((prop) => ({ prop, value: values[prop.id] }))
		.filter((entry) => entry.value !== undefined);
	if (overrides.length === 0) return components;

	const touched = new Set(
		overrides.map((entry) => entry.prop.targetComponentId),
	);
	const next = components.map((component) =>
		touched.has(component.id) ? (structuredClone(component) as T) : component,
	);
	const byId = new Map(next.map((component) => [component.id, component]));
	for (const { prop, value } of overrides) {
		const target = byId.get(prop.targetComponentId);
		if (!target) continue;
		const parts = prop.propertyPath.split(".").filter(Boolean);
		if (parts.length === 0) continue;
		let root: Record<string, unknown>;
		if (parts[0] === "style") {
			root = target as unknown as Record<string, unknown>;
		} else {
			if (!target.component) target.component = {};
			root = target.component;
		}
		setDeep(root, parts, shapeExposedValue(value, prop.propType));
	}
	return next;
}

export interface WidgetInstanceComponentProps {
	widgetId: string;
	instanceId: string;
	appId?: string;
	exposedPropValues?: Record<string, unknown>;
	styleOverride?: Record<string, unknown>;
	actionBindings?: Record<string, ActionBinding>;
	actions?: Action[];
	eventHandlers?: EventHandlers;
	style?: Style;
	inlineWidgetDef?: InlineWidgetDef;
	runtimeChildUpdates?: Record<string, Record<string, unknown>[]>;
}

export interface WidgetInstanceContextValue {
	instanceId: string;
	widgetId: string;
	actionBindings: Record<string, ActionBinding>;
	/** Outer declarative component id used while executing its configured actions. */
	componentId?: string;
	/** Only declarative widget instances expose these; micro widgets route them first. */
	actions?: Action[];
	eventHandlers?: EventHandlers;
	/** Resolved children included in this instance's workflow callback payload. */
	components?: readonly WidgetComponentDef[];
}

const WidgetInstanceContext = createContext<WidgetInstanceContextValue | null>(
	null,
);

export function useWidgetInstance(): WidgetInstanceContextValue | null {
	return useContext(WidgetInstanceContext);
}

export type WidgetInstanceEventRoute =
	| { kind: "actions"; actions: Action[] }
	| { kind: "binding"; binding: ActionBinding }
	| { kind: "diagnostic" };

/**
 * Resolve a declarative widget event in compatibility order. Named handlers
 * (including an explicit empty list) override classic instance bindings;
 * legacy component actions are considered only when no binding exists.
 */
export function resolveWidgetInstanceEventRoute(
	widgetInstance: WidgetInstanceContextValue | null,
	actionId: string,
): WidgetInstanceEventRoute {
	const named = resolveEventActions(
		widgetInstance?.eventHandlers,
		actionId,
		undefined,
		{ legacyFallback: false },
	);
	if (named.source !== "none") {
		return { kind: "actions", actions: named.actions };
	}

	const binding = widgetInstance?.actionBindings[actionId];
	if (binding) return { kind: "binding", binding };

	const legacyAction = widgetInstance?.actions?.[0];
	if (legacyAction) return { kind: "actions", actions: [legacyAction] };

	return { kind: "diagnostic" };
}

/**
 * Provides the widget-instance scope consumed by `useExecuteAction`'s
 * `widget_event` dispatch (action bindings resolved by actionId). Shared by
 * declarative widget instances and sandboxed micro widget instances.
 */
export function WidgetInstanceProvider({
	instanceId,
	widgetId,
	actionBindings,
	componentId,
	actions,
	eventHandlers,
	components,
	children,
}: {
	instanceId: string;
	widgetId: string;
	actionBindings: Record<string, ActionBinding>;
	componentId?: string;
	actions?: Action[];
	eventHandlers?: EventHandlers;
	components?: readonly WidgetComponentDef[];
	children: React.ReactNode;
}) {
	const value = useMemo(
		() => ({
			instanceId,
			widgetId,
			actionBindings,
			componentId,
			actions,
			eventHandlers,
			components,
		}),
		[
			instanceId,
			widgetId,
			actionBindings,
			componentId,
			actions,
			eventHandlers,
			components,
		],
	);
	return (
		<WidgetInstanceContext.Provider value={value}>
			{children}
		</WidgetInstanceContext.Provider>
	);
}

/** Replay instance-local updates against a widget definition loaded from widgetRefs. */
export function applyRuntimeChildUpdates<T extends WidgetComponentDef>(
	components: T[],
	updates: Record<string, Record<string, unknown>[]> | undefined,
): T[] {
	if (!updates || Object.keys(updates).length === 0) return components;

	let next = components;
	let changed = false;
	for (const [requestedId, operations] of Object.entries(updates)) {
		for (const operation of operations) {
			let index = next.findIndex((component) => component.id === requestedId);
			if (index < 0 && operation.type !== "createComponent") {
				const suffix = `-${requestedId}`;
				index = next.findIndex((component) => component.id.endsWith(suffix));
			}
			if (index < 0) continue;

			if (!changed) {
				next = [...components];
				changed = true;
			}

			const current = next[index];
			if (operation.type === "createComponent") {
				next[index] = {
					...current,
					component: operation.component as Record<string, unknown>,
					style: operation.style ?? current.style,
				} as T;
				continue;
			}

			const updated = applyElementUpdate(
				{
					id: current.id,
					component: current.component as unknown as A2UIComponent,
					style: current.style as Style | undefined,
				},
				operation,
			);
			next[index] = {
				...current,
				component: updated.component as unknown as Record<string, unknown>,
				style: updated.style,
			} as T;
		}
	}

	return next;
}

/**
 * A2UIWidgetInstance renders a widget instance by looking up the widget definition
 * from widgetRefs (stored on the page) and rendering its component tree.
 */
export function A2UIWidgetInstance({
	component,
	componentId,
	appId: rendererAppId,
	boardId,
	onAction,
	renderChild: renderSurfaceChild,
}: ComponentProps) {
	const { t } = useTranslation("common");
	const props = component as unknown as WidgetInstanceComponentProps;
	const {
		instanceId,
		widgetId,
		appId,
		inlineWidgetDef,
		actionBindings,
		exposedPropValues,
		runtimeChildUpdates,
	} = props;
	const effectiveAppId = appId ?? rendererAppId;
	const widgetRefsContext = useWidgetRefs();
	const backend = useBackend();
	const { resolve } = useData();

	const fromRefs = widgetRefsContext?.getWidgetRef(instanceId);

	const shouldFetch =
		!fromRefs && !inlineWidgetDef && !!effectiveAppId && !!widgetId;
	const fetched = useInvoke(
		backend.widgetState.getWidget,
		backend.widgetState,
		[effectiveAppId ?? "", widgetId],
		shouldFetch,
	);

	const widgetDef = useMemo(() => {
		if (fromRefs) return fromRefs;
		if (inlineWidgetDef) return inlineWidgetDef;
		return fetched.data;
	}, [fromRefs, inlineWidgetDef, fetched.data]);

	// Apply this instance's parameter values onto the widget's components, so the same widget
	// definition can render differently per instance (e.g. a stat card with a per-instance title).
	const resolvedWidgetDef = useMemo(() => {
		if (!widgetDef) return widgetDef;
		const exposedProps =
			(widgetDef as { exposedProps?: ExposedPropDef[] }).exposedProps ?? [];
		const original = widgetDef.components as WidgetComponentDef[];
		const applied = applyExposedProps(
			original,
			exposedProps,
			exposedPropValues,
		);
		if (applied === original) return widgetDef;
		return { ...widgetDef, components: applied } as typeof widgetDef;
	}, [widgetDef, exposedPropValues]);

	const renderedWidgetDef = useMemo(() => {
		const base = resolvedWidgetDef ?? widgetDef;
		if (!base) return base;
		const original = base.components as WidgetComponentDef[];
		const updated = applyRuntimeChildUpdates(original, runtimeChildUpdates);
		if (updated === original) return base;
		return { ...base, components: updated } as typeof base;
	}, [resolvedWidgetDef, widgetDef, runtimeChildUpdates]);

	// Create a local renderChild for the widget's internal components
	const renderWidgetChild = useCallback(
		(childId: string, currentWidgetDef: typeof widgetDef): React.ReactNode => {
			if (!currentWidgetDef) return null;

			const childComponent = currentWidgetDef.components.find(
				(c) => c.id === childId,
			);
			if (!childComponent?.component) {
				// A child pushed in at runtime — another widget instance, or any
				// element created on the page — lives on the surface, not in this
				// widget's definition. Push Child stored its surface id in the
				// container's explicitList, so resolve it through the page renderer.
				// The self-reference guard keeps a container pushed into itself from
				// recursing forever.
				if (childId !== instanceId && childId !== componentId) {
					const external = renderSurfaceChild?.(childId);
					if (external) return external;
				}
				console.warn(
					`Widget "${currentWidgetDef.name}" component "${childId}" not found. Available components:`,
					currentWidgetDef.components.map((c) => c.id),
				);
				return null;
			}

			if (resolveHidden(childComponent.component.hidden, resolve)) return null;

			const componentType = childComponent.component.type as string;
			const Renderer = getComponentRenderer(componentType);
			if (!Renderer) {
				console.warn(`Unknown component type: ${componentType}`);
				return null;
			}

			return (
				<Renderer
					key={childId}
					component={childComponent.component as A2UIComponent}
					componentId={childId}
					surfaceId={instanceId}
					appId={effectiveAppId}
					boardId={boardId}
					style={
						childComponent.style ??
						(childComponent.component.style as Style | undefined)
					}
					onAction={onAction}
					renderChild={(nestedChildId) =>
						renderWidgetChild(nestedChildId, currentWidgetDef)
					}
				/>
			);
		},
		[
			instanceId,
			componentId,
			effectiveAppId,
			boardId,
			onAction,
			resolve,
			renderSurfaceChild,
		],
	);

	if (!renderedWidgetDef) {
		if (shouldFetch && (fetched.isLoading || fetched.isFetching)) {
			return (
				<div
					className="p-4 text-sm text-muted-foreground"
					data-widget-instance={instanceId}
					data-widget-id={widgetId}
				>
					{t("loadingWidget", "Loading widget…")}
				</div>
			);
		}
		return (
			<div className="p-4 text-sm text-red-500 bg-red-50 rounded">
				{t(
					"widgetInstanceQuotinstanceidquotCouldNotBeResolved",
					'Widget instance "{{instanceId}}" could not be resolved',
					{ instanceId },
				)}
				{fetched.error ? `: ${fetched.error.message}` : ""}
			</div>
		);
	}

	if (!renderedWidgetDef.rootComponentId) {
		return (
			<div className="p-4 text-sm text-red-500 bg-red-50 rounded">
				{t(
					"widgetDefinitionMissingRootcomponentid",
					"Widget definition missing rootComponentId",
				)}
			</div>
		);
	}

	return (
		<WidgetInstanceProvider
			instanceId={instanceId}
			widgetId={widgetId}
			actionBindings={actionBindings ?? {}}
			componentId={componentId}
			actions={props.actions}
			eventHandlers={props.eventHandlers}
			components={renderedWidgetDef.components as WidgetComponentDef[]}
		>
			<div
				data-widget-instance={instanceId}
				data-widget-id={widgetId}
				className="contents"
			>
				{renderWidgetChild(
					renderedWidgetDef.rootComponentId,
					renderedWidgetDef,
				)}
			</div>
		</WidgetInstanceProvider>
	);
}
