import type { Style, SurfaceComponent } from "./types";

type JsonRecord = Record<string, unknown>;

function isRecord(value: unknown): value is JsonRecord {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function splitCssShorthand(value: string): string[] {
	const parts: string[] = [];
	let current = "";
	let depth = 0;

	for (const character of value.trim()) {
		if (character === "(") depth += 1;
		if (character === ")") depth = Math.max(0, depth - 1);

		if (/\s/.test(character) && depth === 0) {
			if (current) {
				parts.push(current);
				current = "";
			}
			continue;
		}
		current += character;
	}

	if (current) parts.push(current);
	return parts;
}

function spacingFromShorthand(value: string): JsonRecord {
	const parts = splitCssShorthand(value);
	if (parts.length === 0) return {};

	const [first, second = first, third = first, fourth = second] = parts;
	if (parts.length === 2) {
		return { top: first, right: second, bottom: first, left: second };
	}
	if (parts.length === 3) {
		return { top: first, right: second, bottom: third, left: second };
	}
	return {
		top: first,
		right: second,
		bottom: third,
		left: fourth,
	};
}

function normalizeSpacing(value: unknown): unknown {
	if (typeof value === "string") return spacingFromShorthand(value);
	if (!isRecord(value)) return value;
	if (typeof value.value === "string") return spacingFromShorthand(value.value);

	const result: JsonRecord = {};
	for (const edge of ["top", "right", "bottom", "left"] as const) {
		if (typeof value[edge] === "string") result[edge] = value[edge];
	}
	return result;
}

function normalizeSize(value: unknown): unknown {
	if (isRecord(value) && typeof value.value === "string") return value.value;
	return value;
}

function normalizeGradient(value: unknown): unknown {
	if (!isRecord(value)) return value;

	const { gradientType: _legacyGradientType, ...normalized } = value;
	const gradientType = value.type ?? value.gradientType;
	if (typeof gradientType === "string") normalized.type = gradientType;

	if (typeof value.direction === "string") {
		normalized.direction = value.direction;
		if (
			typeof value.angle !== "number" &&
			/^-?(?:\d+|\d*\.\d+)deg$/.test(value.direction.trim())
		) {
			normalized.angle = Number.parseFloat(value.direction);
		}
	}

	if (Array.isArray(value.stops)) {
		const usesLegacyFractionalStops =
			value.type === undefined && typeof value.gradientType === "string";
		normalized.stops = value.stops.map((stop) => {
			if (!isRecord(stop)) return stop;
			return {
				...stop,
				position:
					usesLegacyFractionalStops &&
					typeof stop.position === "number" &&
					stop.position >= 0 &&
					stop.position <= 1
						? stop.position * 100
						: stop.position,
			};
		});
	}

	return normalized;
}

function normalizeBackground(value: unknown): unknown {
	if (!isRecord(value)) return value;
	if ("gradient" in value) {
		return { ...value, gradient: normalizeGradient(value.gradient) };
	}
	return { ...value };
}

function normalizePosition(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const { positionType: _legacyPositionType, ...normalized } = value;
	const positionType = value.type ?? value.positionType;
	if (typeof positionType === "string") normalized.type = positionType;
	return normalized;
}

function normalizeBreakpoint(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const normalized: JsonRecord = { ...value };
	for (const key of ["width", "height"] as const) {
		if (key in normalized) normalized[key] = normalizeSize(normalized[key]);
	}
	for (const key of ["padding", "margin"] as const) {
		if (key in normalized) normalized[key] = normalizeSpacing(normalized[key]);
	}
	return normalized;
}

function normalizeResponsive(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const normalized: JsonRecord = {};
	for (const [breakpoint, breakpointStyle] of Object.entries(value)) {
		normalized[breakpoint] = normalizeBreakpoint(breakpointStyle);
	}
	return normalized;
}

/**
 * Converts compatibility aliases produced by older Rust serializers into the
 * established frontend A2UI style shape. The function is pure and preserves
 * unknown properties so newly added style fields are not lost in transit.
 */
export function normalizeStyleForPersistence(
	style: unknown,
): Style | undefined {
	if (!isRecord(style)) return undefined;
	const { responsive: legacyResponsive, ...normalized } = style;

	if ("background" in normalized) {
		normalized.background = normalizeBackground(normalized.background);
	}
	if ("position" in normalized) {
		normalized.position = normalizePosition(normalized.position);
	}
	for (const key of ["margin", "padding"] as const) {
		if (key in normalized) normalized[key] = normalizeSpacing(normalized[key]);
	}
	for (const key of [
		"width",
		"height",
		"minWidth",
		"minHeight",
		"maxWidth",
		"maxHeight",
	] as const) {
		if (key in normalized) normalized[key] = normalizeSize(normalized[key]);
	}

	const responsive = normalized.responsiveOverrides ?? legacyResponsive;
	if (responsive !== undefined) {
		normalized.responsiveOverrides = normalizeResponsive(responsive);
	}

	return normalized as Style;
}

function normalizeInlineWidgetDefinition(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const normalized: JsonRecord = { ...value };
	if (Array.isArray(value.components)) {
		normalized.components = normalizeSurfaceComponentsForPersistence(
			value.components as SurfaceComponent[],
		);
	}
	return normalized;
}

export function normalizeSurfaceComponentForPersistence<
	T extends SurfaceComponent,
>(value: T): T {
	const normalized = { ...value } as T & JsonRecord;
	if (value.style) normalized.style = normalizeStyleForPersistence(value.style);

	if (isRecord(value.component)) {
		const component: JsonRecord = { ...value.component };
		for (const styleKey of [
			"style",
			"styleOverride",
			"listStyle",
			"triggerStyle",
			"contentStyle",
		] as const) {
			if (component[styleKey]) {
				component[styleKey] = normalizeStyleForPersistence(component[styleKey]);
			}
		}
		if (component.inlineWidgetDef) {
			component.inlineWidgetDef = normalizeInlineWidgetDefinition(
				component.inlineWidgetDef,
			);
		}
		normalized.component = component as unknown as T["component"];
	}

	return normalized;
}

export function normalizeSurfaceComponentsForPersistence<
	T extends SurfaceComponent,
>(components: readonly T[]): T[] {
	return components.map(normalizeSurfaceComponentForPersistence);
}

export function normalizeWidgetForPersistence<
	T extends { components: SurfaceComponent[] },
>(widget: T): T {
	return {
		...widget,
		components: normalizeSurfaceComponentsForPersistence(widget.components),
	};
}

function normalizePageContent(value: unknown): unknown {
	if (!isRecord(value)) return value;
	if (isRecord(value.Component)) {
		return {
			...value,
			Component: normalizeSurfaceComponentForPersistence(
				value.Component as unknown as SurfaceComponent,
			),
		};
	}
	if (isRecord(value.Widget)) {
		const widget = { ...value.Widget };
		if (widget.styleOverride) {
			widget.styleOverride = normalizeStyleForPersistence(widget.styleOverride);
		}
		return { ...value, Widget: widget };
	}
	return { ...value };
}

export function normalizePageForPersistence<
	T extends {
		components: SurfaceComponent[];
		content?: unknown[];
		widgetRefs?: Record<string, { components: SurfaceComponent[] }>;
	},
>(page: T): T {
	const widgetRefs = page.widgetRefs
		? Object.fromEntries(
				Object.entries(page.widgetRefs).map(([id, widget]) => [
					id,
					normalizeWidgetForPersistence(widget),
				]),
			)
		: page.widgetRefs;

	return {
		...page,
		components: normalizeSurfaceComponentsForPersistence(page.components),
		...(page.content
			? { content: page.content.map(normalizePageContent) }
			: {}),
		...(widgetRefs ? { widgetRefs } : {}),
	};
}
