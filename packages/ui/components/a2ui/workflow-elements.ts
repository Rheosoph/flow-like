import { collectMicroWidgetValueKeys } from "./micro-widget-host";
import type { SurfaceComponent } from "./types";

export interface WidgetElementComponent {
	id: string;
	component?: unknown;
	style?: unknown;
	eventRelevant?: boolean;
}

/** The widget instance whose event is currently executing. */
export interface WidgetElementScope {
	instanceId: string;
	components?: readonly WidgetElementComponent[];
}

function componentData(component: SurfaceComponent): Record<string, unknown> {
	return component.component as unknown as Record<string, unknown>;
}

function isWidgetHost(data: Record<string, unknown>): boolean {
	return data.type === "widgetInstance" || data.type === "microWidgetInstance";
}

/** Storage prefixes owned by a surface, including each hosted widget instance. */
export function elementValueScopeIds(
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
): string[] {
	const ids = new Set<string>();
	if (surfaceId) ids.add(surfaceId);

	for (const component of Object.values(components ?? {})) {
		const data = componentData(component);
		if (!isWidgetHost(data) || typeof data.instanceId !== "string") continue;
		if (data.instanceId) ids.add(data.instanceId);
	}

	return [...ids];
}

function inlineWidgetComponents(
	data: Record<string, unknown>,
): readonly WidgetElementComponent[] {
	const inlineDefinition = data.inlineWidgetDef as
		| { components?: WidgetElementComponent[] }
		| undefined;
	return inlineDefinition?.components ?? [];
}

export function widgetComponentsForScope(
	components: Record<string, SurfaceComponent> | undefined,
	widgetScope: WidgetElementScope,
): readonly WidgetElementComponent[] {
	if (widgetScope.components) return widgetScope.components;
	if (!components) return [];

	for (const component of Object.values(components)) {
		const data = componentData(component);
		if (
			data.type === "widgetInstance" &&
			data.instanceId === widgetScope.instanceId
		) {
			return inlineWidgetComponents(data);
		}
	}
	return [];
}

function payloadElement(
	component: WidgetElementComponent,
): Record<string, unknown> {
	return {
		...component,
		id: component.id,
		component: component.component ?? null,
	};
}

/**
 * Flatten the rendered surface into the `_elements` map used by workflow reads.
 *
 * A normal page event retains the legacy `surfaceId/childId` representation. A
 * widget event includes only the triggering widget host and addresses its
 * children as `instanceId/childId`, so repeated widget instances remain
 * distinguishable without sending every widget definition to the backend.
 */
export function flattenSurfaceComponentsForElements(
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
	widgetScope?: WidgetElementScope,
): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	if (!components || !surfaceId) return result;

	for (const [hostComponentId, component] of Object.entries(components)) {
		const data = componentData(component);
		const widgetHost = isWidgetHost(data);

		if (widgetScope && widgetHost) {
			if (data.instanceId !== widgetScope.instanceId) continue;

			result[`${surfaceId}/${hostComponentId}`] = component;
			if (data.type !== "widgetInstance") continue;

			const children = widgetComponentsForScope(components, widgetScope);
			for (const child of children) {
				if (!child?.id) continue;
				result[`${widgetScope.instanceId}/${child.id}`] = payloadElement(child);
			}
			continue;
		}

		result[`${surfaceId}/${hostComponentId}`] = component;
		if (widgetScope || data.type !== "widgetInstance") continue;

		for (const child of inlineWidgetComponents(data)) {
			if (!child?.id) continue;
			if (typeof data.instanceId === "string" && data.instanceId) {
				result[`${data.instanceId}/${child.id}`] = payloadElement(child);
			}
			const legacyKey = `${surfaceId}/${child.id}`;
			if (result[legacyKey] === undefined) {
				result[legacyKey] = payloadElement(child);
			}
		}
	}

	return result;
}

function storedValueForElement(
	storedValues: Record<string, unknown>,
	elementId: string,
	widgetScope: WidgetElementScope | undefined,
	legacySurfaceId: string | undefined,
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
): unknown {
	const exact = storedValues[elementId];
	if (exact !== undefined) return exact;

	if (!widgetScope) {
		const surfacePrefix = `${surfaceId}/`;
		if (!elementId.startsWith(surfacePrefix)) return undefined;
		const childId = elementId.slice(surfacePrefix.length);
		if (!childId || components?.[childId]) return undefined;

		const instanceIds = Object.values(components ?? {}).flatMap((component) => {
			const data = componentData(component);
			if (
				data.type !== "widgetInstance" ||
				typeof data.instanceId !== "string" ||
				!inlineWidgetComponents(data).some((child) => child.id === childId)
			) {
				return [];
			}
			return [data.instanceId];
		});
		if (instanceIds.length !== 1) return undefined;
		return storedValues[`${instanceIds[0]}/${childId}`];
	}

	if (!legacySurfaceId) return undefined;

	const instancePrefix = `${widgetScope.instanceId}/`;
	if (!elementId.startsWith(instancePrefix)) return undefined;

	// Compatibility for values recorded before widget children received an
	// instance-scoped surface id. This is disabled when multiple declarative
	// widgets share a surface because the old key cannot identify its owner.
	const childId = elementId.slice(instancePrefix.length);
	return storedValues[`${legacySurfaceId}/${childId}`];
}

export function legacyWidgetValueSurfaceId(
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
	widgetScope: WidgetElementScope | undefined,
): string | undefined {
	if (!widgetScope || !surfaceId) return undefined;
	const declarativeWidgetCount = Object.values(components ?? {}).filter(
		(component) => componentData(component).type === "widgetInstance",
	).length;
	return declarativeWidgetCount === 1 ? surfaceId : undefined;
}

function filterElementsForWidgetScope(
	elementsMap: Record<string, unknown>,
	components: Record<string, SurfaceComponent> | undefined,
	widgetScope: WidgetElementScope | undefined,
): Record<string, unknown> {
	if (!widgetScope) return elementsMap;

	const otherInstanceIds = new Set<string>();
	for (const component of Object.values(components ?? {})) {
		const data = componentData(component);
		if (
			isWidgetHost(data) &&
			typeof data.instanceId === "string" &&
			data.instanceId !== widgetScope.instanceId
		) {
			otherInstanceIds.add(data.instanceId);
		}
	}

	return Object.fromEntries(
		Object.entries(elementsMap).filter(([elementId, element]) => {
			const separator = elementId.indexOf("/");
			if (
				separator >= 0 &&
				otherInstanceIds.has(elementId.slice(0, separator))
			) {
				return false;
			}

			if (!element || typeof element !== "object" || Array.isArray(element)) {
				return true;
			}
			const data = (element as Record<string, unknown>).component;
			if (!data || typeof data !== "object" || Array.isArray(data)) return true;
			const component = data as Record<string, unknown>;
			return (
				!isWidgetHost(component) ||
				component.instanceId === widgetScope.instanceId
			);
		}),
	);
}

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

/** Merge persisted input values into a workflow element snapshot. */
export function mergeStoredElementValues(
	elementsMap: Record<string, unknown>,
	storedValues: Record<string, unknown>,
	components: Record<string, SurfaceComponent> | undefined,
	surfaceId: string,
	widgetScope?: WidgetElementScope,
): Record<string, unknown> {
	const flattened = flattenSurfaceComponentsForElements(
		components,
		surfaceId,
		widgetScope,
	);
	const filteredElements = filterElementsForWidgetScope(
		elementsMap,
		components,
		widgetScope,
	);
	const mergedElements: Record<string, unknown> = {
		...filteredElements,
		...flattened,
	};
	const legacySurfaceId = legacyWidgetValueSurfaceId(
		components,
		surfaceId,
		widgetScope,
	);

	for (const [elementId, element] of Object.entries(mergedElements)) {
		const storedValue = storedValueForElement(
			storedValues,
			elementId,
			widgetScope,
			legacySurfaceId,
			components,
			surfaceId,
		);
		if (storedValue === undefined || !element || typeof element !== "object") {
			continue;
		}

		const payload = element as Record<string, unknown>;
		const data = payload.component;
		if (!data || typeof data !== "object" || Array.isArray(data)) continue;

		mergedElements[elementId] = {
			...payload,
			component: {
				...(data as Record<string, unknown>),
				value: toBoundValue(storedValue),
			},
		};
	}

	const microValueKeys = widgetScope
		? new Set([`${widgetScope.instanceId}/values`])
		: collectMicroWidgetValueKeys(components);
	const allowedPrefixes = widgetScope
		? [`${widgetScope.instanceId}/`]
		: elementValueScopeIds(components, surfaceId).map((id) => `${id}/`);

	for (const [elementId, storedValue] of Object.entries(storedValues)) {
		if (mergedElements[elementId] !== undefined) continue;
		const isMicroValues = microValueKeys.has(elementId);
		const allowedByScope = allowedPrefixes.some((prefix) =>
			elementId.startsWith(prefix),
		);
		if (!isMicroValues && !allowedByScope) continue;

		const separator = elementId.indexOf("/");
		mergedElements[elementId] = {
			id: separator >= 0 ? elementId.slice(separator + 1) : elementId,
			component: { value: toBoundValue(storedValue) },
		};
	}

	return mergedElements;
}

/** Build `_input_values` for the active surface or widget instance. */
export function collectEventRelevantInputValues(
	storedValues: Record<string, unknown>,
	components: readonly WidgetElementComponent[],
	scopeId: string,
	legacySurfaceId?: string,
): Record<string, unknown> {
	const inputValues: Record<string, unknown> = {};
	for (const component of components) {
		if (!component.eventRelevant) continue;
		const scopedValue = storedValues[`${scopeId}/${component.id}`];
		const value =
			scopedValue !== undefined
				? scopedValue
				: legacySurfaceId
					? storedValues[`${legacySurfaceId}/${component.id}`]
					: undefined;
		if (value !== undefined) inputValues[component.id] = value;
	}
	return inputValues;
}
