import type { Style } from "./types";

const STYLE_KEY_ALIASES: Record<string, keyof Style> = {
	align_self: "alignSelf",
	backdrop_filter: "backdropFilter",
	class_name: "className",
	classname: "className",
	flex_basis: "flexBasis",
	flex_grow: "flexGrow",
	flex_shrink: "flexShrink",
	font_family: "fontFamily",
	font_size: "fontSize",
	font_weight: "fontWeight",
	grid_area: "gridArea",
	grid_column: "gridColumn",
	grid_row: "gridRow",
	justify_self: "justifySelf",
	letter_spacing: "letterSpacing",
	line_height: "lineHeight",
	max_height: "maxHeight",
	max_width: "maxWidth",
	min_height: "minHeight",
	min_width: "minWidth",
	outline_offset: "outlineOffset",
	pointer_events: "pointerEvents",
	responsive: "responsiveOverrides",
	responsive_overrides: "responsiveOverrides",
	text_align: "textAlign",
	text_decoration: "textDecoration",
	text_transform: "textTransform",
	user_select: "userSelect",
	white_space: "whiteSpace",
	word_break: "wordBreak",
	z_index: "zIndex",
};

function isRecord(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasBaseOpacityUtility(value: unknown): boolean {
	if (typeof value !== "string") return false;
	return value
		.split(/\s+/)
		.some((className) => className.startsWith("opacity-"));
}

function splitCssShorthand(value: string): string[] {
	const parts: string[] = [];
	let current = "";
	let depth = 0;
	let quote: "'" | '"' | undefined;

	for (const character of value.trim()) {
		if (quote) {
			current += character;
			if (character === quote) quote = undefined;
			continue;
		}
		if (character === "'" || character === '"') {
			quote = character;
			current += character;
			continue;
		}
		if (character === "(") depth += 1;
		if (character === ")") depth = Math.max(0, depth - 1);
		if (/\s/.test(character) && depth === 0) {
			if (current) parts.push(current);
			current = "";
			continue;
		}
		current += character;
	}
	if (current) parts.push(current);
	return parts;
}

function normalizeSpacing(value: unknown): unknown {
	if (!isRecord(value) || typeof value.value !== "string") return value;
	const parts = splitCssShorthand(value.value);
	if (parts.length === 0) return {};
	const [top, second = top, third = top, fourth = second] = parts;
	return {
		top,
		right: second,
		bottom: third,
		left: fourth,
	};
}

function normalizeStyleValue(value: unknown): unknown {
	return isRecord(value) && typeof value.value === "string"
		? value.value
		: value;
}

function normalizeBackground(value: unknown): unknown {
	if (!isRecord(value) || !isRecord(value.gradient)) return value;
	const gradient = value.gradient;
	const type = gradient.type ?? gradient.gradientType;
	const legacyFractionalStops =
		gradient.type === undefined && gradient.gradientType !== undefined;
	const stops = Array.isArray(gradient.stops)
		? gradient.stops.map((stop) => {
				if (!isRecord(stop)) return stop;
				const position = stop.position;
				return legacyFractionalStops &&
					typeof position === "number" &&
					position >= 0 &&
					position <= 1
					? { ...stop, position: position * 100 }
					: stop;
			})
		: gradient.stops;
	const { gradientType: _gradientType, ...rest } = gradient;
	return {
		...value,
		gradient: { ...rest, type, stops },
	};
}

function normalizePosition(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const { positionType, ...rest } = value;
	return { ...rest, type: value.type ?? positionType };
}

function normalizeBreakpoint(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const normalized: Record<string, unknown> = { ...value };
	for (const key of ["width", "height"] as const) {
		normalized[key] = normalizeStyleValue(normalized[key]);
	}
	for (const key of ["padding", "margin"] as const) {
		normalized[key] = normalizeSpacing(normalized[key]);
	}
	return normalized;
}

function normalizeResponsive(value: unknown): unknown {
	if (!isRecord(value)) return value;
	const normalized: Record<string, unknown> = {};
	for (const [breakpoint, breakpointStyle] of Object.entries(value)) {
		normalized[breakpoint] = normalizeBreakpoint(breakpointStyle);
	}
	return normalized;
}

function normalizeStyleField(key: keyof Style, value: unknown): unknown {
	switch (key) {
		case "background":
			return normalizeBackground(value);
		case "position":
			return normalizePosition(value);
		case "margin":
		case "padding":
			return normalizeSpacing(value);
		case "width":
		case "height":
		case "minWidth":
		case "minHeight":
		case "maxWidth":
		case "maxHeight":
			return normalizeStyleValue(value);
		case "responsiveOverrides":
			return normalizeResponsive(value);
		default:
			return value;
	}
}

export function normalizeStyleUpdate(update: unknown): Partial<Style> {
	if (!isRecord(update)) return {};

	const normalized: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(update)) {
		const normalizedKey = STYLE_KEY_ALIASES[key] ?? key;
		if (
			normalizedKey !== key &&
			Object.prototype.hasOwnProperty.call(update, normalizedKey)
		) {
			continue;
		}
		normalized[normalizedKey] = normalizeStyleField(
			normalizedKey as keyof Style,
			value,
		);
	}

	return normalized as Partial<Style>;
}

export function applyStyleUpdate(
	currentStyle: Style | undefined,
	update: unknown,
): Style {
	const next: Record<string, unknown> = isRecord(currentStyle)
		? { ...normalizeStyleUpdate(currentStyle) }
		: {};
	const normalized = normalizeStyleUpdate(update);

	for (const [key, value] of Object.entries(normalized)) {
		if (value == null) {
			delete next[key];
		} else {
			next[key] = value;
		}
	}

	if (
		!Object.prototype.hasOwnProperty.call(normalized, "opacity") &&
		hasBaseOpacityUtility(normalized.className)
	) {
		next.opacity = undefined;
	}

	return next as Style;
}
