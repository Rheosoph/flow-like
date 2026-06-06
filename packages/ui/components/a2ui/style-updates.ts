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
	return value.split(/\s+/).some((className) => className.startsWith("opacity-"));
}

export function normalizeStyleUpdate(update: unknown): Partial<Style> {
	if (!isRecord(update)) return {};

	const normalized: Record<string, unknown> = {};
	for (const [key, value] of Object.entries(update)) {
		const normalizedKey = STYLE_KEY_ALIASES[key] ?? key;
		normalized[normalizedKey] = value;
	}

	return normalized as Partial<Style>;
}

export function applyStyleUpdate(
	currentStyle: Style | undefined,
	update: unknown,
): Style {
	const next: Record<string, unknown> = isRecord(currentStyle)
		? { ...currentStyle }
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
		delete next.opacity;
	}

	return next as Style;
}
