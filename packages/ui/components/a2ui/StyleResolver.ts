import { cn } from "../../lib/utils";
import type {
	Background,
	Border,
	BreakpointStyle,
	Overflow,
	Position,
	ResponsiveOverrides,
	Shadow,
	Spacing,
	Style,
	StyleValue,
	Transform,
} from "./types";

export function resolveStyle(style: Style | undefined): string {
	if (!style) return "";

	const classes: string[] = [];

	// Primary class names (Tailwind-first approach)
	if (style.className) {
		classes.push(style.className);
	}

	// Overflow
	if (style.overflow) {
		classes.push(overflowToClass(style.overflow));
	}

	// Responsive overrides
	const responsive = style.responsiveOverrides ?? style.responsive;
	if (responsive) {
		classes.push(...resolveResponsiveOverrides(responsive));
	}

	return cn(...classes);
}

function overflowToClass(overflow: Overflow): string {
	switch (overflow) {
		case "visible":
			return "overflow-visible";
		case "hidden":
			return "overflow-hidden";
		case "scroll":
			return "overflow-scroll";
		case "auto":
			return "overflow-auto";
		default:
			return "";
	}
}

function resolveResponsiveOverrides(overrides: ResponsiveOverrides): string[] {
	const classes: string[] = [];
	const breakpoints = ["sm", "md", "lg", "xl", "xxl"] as const;
	const properties: Array<keyof BreakpointStyle> = [
		"display",
		"flexDirection",
		"justifyContent",
		"alignItems",
		"gap",
		"gridCols",
		"width",
		"height",
		"padding",
		"margin",
		"fontSize",
		"textAlign",
		"order",
	];

	for (const breakpoint of breakpoints) {
		const override = overrides[breakpoint];
		if (!override) continue;
		const tailwindPrefix = breakpoint === "xxl" ? "2xl" : breakpoint;
		if (override.className) {
			classes.push(
				...override.className
					.split(/\s+/)
					.filter(Boolean)
					.map((className) => `${tailwindPrefix}:${className}`),
			);
		}

		for (const property of properties) {
			if (override[property] !== undefined) {
				classes.push(`a2ui-${breakpoint}-${toKebabCase(property)}`);
			}
		}
		if (override.hidden) classes.push(`a2ui-${breakpoint}-display`);
	}

	return classes;
}

export function resolveInlineStyle(
	style: Style | undefined,
): React.CSSProperties {
	if (!style) return {};

	const inlineStyle: React.CSSProperties = {};

	// Background
	if (style.background) {
		Object.assign(inlineStyle, backgroundToCss(style.background));
	}

	// Border
	if (style.border) {
		Object.assign(inlineStyle, borderToCss(style.border));
	}

	// Shadow
	if (style.shadow) {
		Object.assign(inlineStyle, shadowToCss(style.shadow));
	}

	// Position
	if (style.position) {
		Object.assign(inlineStyle, positionToCss(style.position));
	}

	// Transform
	if (style.transform) {
		Object.assign(inlineStyle, transformToCss(style.transform));
	}

	// Spacing (margin & padding)
	if (style.margin) {
		Object.assign(inlineStyle, spacingToCss(style.margin, "margin"));
	}
	if (style.padding) {
		Object.assign(inlineStyle, spacingToCss(style.padding, "padding"));
	}
	if (style.gap) inlineStyle.gap = style.gap;

	// Sizing
	if (style.width) inlineStyle.width = styleValueToCss(style.width);
	if (style.height) inlineStyle.height = styleValueToCss(style.height);
	if (style.minWidth) inlineStyle.minWidth = styleValueToCss(style.minWidth);
	if (style.minHeight) inlineStyle.minHeight = styleValueToCss(style.minHeight);
	if (style.maxWidth) inlineStyle.maxWidth = styleValueToCss(style.maxWidth);
	if (style.maxHeight) inlineStyle.maxHeight = styleValueToCss(style.maxHeight);

	// Flex item properties
	if (style.flex) inlineStyle.flex = style.flex;
	if (style.flexGrow !== undefined) inlineStyle.flexGrow = style.flexGrow;
	if (style.flexShrink !== undefined) inlineStyle.flexShrink = style.flexShrink;
	if (style.flexBasis) inlineStyle.flexBasis = style.flexBasis;
	if (style.alignSelf) inlineStyle.alignSelf = style.alignSelf;

	// Grid item properties
	if (style.gridColumn) inlineStyle.gridColumn = style.gridColumn;
	if (style.gridRow) inlineStyle.gridRow = style.gridRow;
	if (style.gridArea) inlineStyle.gridArea = style.gridArea;
	if (style.justifySelf) inlineStyle.justifySelf = style.justifySelf;

	// Typography
	if (style.color) inlineStyle.color = style.color;
	if (style.fontSize) inlineStyle.fontSize = style.fontSize;
	if (style.fontWeight) inlineStyle.fontWeight = style.fontWeight;
	if (style.fontFamily) inlineStyle.fontFamily = style.fontFamily;
	if (style.lineHeight) inlineStyle.lineHeight = style.lineHeight;
	if (style.letterSpacing) inlineStyle.letterSpacing = style.letterSpacing;
	if (style.textAlign) inlineStyle.textAlign = style.textAlign;
	if (style.textDecoration) inlineStyle.textDecoration = style.textDecoration;
	if (style.textTransform) inlineStyle.textTransform = style.textTransform;
	if (style.whiteSpace) inlineStyle.whiteSpace = style.whiteSpace;
	if (style.wordBreak) inlineStyle.wordBreak = style.wordBreak;

	// Visibility & interaction
	if (style.opacity !== undefined) inlineStyle.opacity = style.opacity;
	if (style.visibility) inlineStyle.visibility = style.visibility;
	if (style.cursor) inlineStyle.cursor = style.cursor;
	if (style.userSelect) inlineStyle.userSelect = style.userSelect;
	if (style.pointerEvents) inlineStyle.pointerEvents = style.pointerEvents;

	// Stacking
	if (style.zIndex !== undefined) inlineStyle.zIndex = style.zIndex;

	// Transitions & animations
	if (style.transition) inlineStyle.transition = style.transition;
	if (style.animation) inlineStyle.animation = style.animation;

	// Display
	if (style.display) inlineStyle.display = style.display;

	// Outline
	if (style.outline) inlineStyle.outline = style.outline;
	if (style.outlineOffset) inlineStyle.outlineOffset = style.outlineOffset;

	// Filters
	if (style.filter) inlineStyle.filter = style.filter;
	if (style.backdropFilter) inlineStyle.backdropFilter = style.backdropFilter;

	// Aspect ratio
	if (style.aspectRatio) inlineStyle.aspectRatio = style.aspectRatio;

	const responsive = style.responsiveOverrides ?? style.responsive;
	if (responsive) {
		Object.assign(inlineStyle, responsiveToCssVariables(responsive));
	}

	return inlineStyle;
}

function backgroundToCss(bg: Background): React.CSSProperties {
	if ("color" in bg) {
		return { backgroundColor: bg.color };
	}
	if ("gradient" in bg) {
		const { gradient } = bg;
		const isLegacyRustShape = gradient.type === undefined;
		const stops = gradient.stops
			.map((s) => {
				const position = gradientStopPositionToCss(
					s.position,
					isLegacyRustShape,
				);
				return `${s.color}${position ? ` ${position}` : ""}`;
			})
			.join(", ");
		const gradientType = gradient.type ?? gradient.gradientType;

		switch (gradientType) {
			case "linear":
				return {
					background: `linear-gradient(${gradient.direction ?? (gradient.angle === undefined ? "180deg" : `${gradient.angle}deg`)}, ${stops})`,
				};
			case "radial":
				return {
					background: `radial-gradient(${gradient.direction ? `${gradient.direction}, ` : ""}${stops})`,
				};
			case "conic":
				return {
					background: `conic-gradient(${gradient.direction ? `${gradient.direction}, ` : gradient.angle === undefined ? "" : `from ${gradient.angle}deg, `}${stops})`,
				};
		}
	}
	if ("image" in bg) {
		const { image } = bg;
		const url =
			"literalString" in image.url
				? image.url.literalString
				: "path" in image.url && typeof image.url.defaultValue === "string"
					? image.url.defaultValue
					: "";
		return {
			backgroundImage: `url(${url})`,
			backgroundSize: image.size ?? "cover",
			backgroundPosition: image.position ?? "center",
			backgroundRepeat: image.repeat ?? "no-repeat",
		};
	}
	if ("blur" in bg) {
		return { backdropFilter: `blur(${bg.blur})` };
	}
	return {};
}

function borderToCss(border: Border): React.CSSProperties {
	const style: React.CSSProperties = {};
	if (border.width) style.borderWidth = border.width;
	if (border.style) style.borderStyle = border.style;
	if (border.color) style.borderColor = border.color;
	if (border.radius) style.borderRadius = border.radius;
	return style;
}

function shadowToCss(shadow: Shadow): React.CSSProperties {
	const hasCanonicalBoxShadow =
		shadow.x !== undefined ||
		shadow.y !== undefined ||
		shadow.blur !== undefined ||
		shadow.spread !== undefined ||
		shadow.color !== undefined ||
		shadow.inset !== undefined;
	if (!hasCanonicalBoxShadow && shadow.boxShadows?.length) {
		return {
			boxShadow: shadow.boxShadows.join(", "),
			textShadow: shadow.textShadow,
		};
	}
	if (!hasCanonicalBoxShadow) {
		return { textShadow: shadow.textShadow };
	}

	const parts = [
		shadow.inset ? "inset" : "",
		shadow.x ?? "0",
		shadow.y ?? "0",
		shadow.blur ?? "0",
		shadow.spread ?? "0",
		shadow.color ?? "rgba(0,0,0,0.25)",
	].filter(Boolean);
	return { boxShadow: parts.join(" "), textShadow: shadow.textShadow };
}

function positionToCss(pos: Position): React.CSSProperties {
	const style: React.CSSProperties = {};
	const positionType = pos.type ?? pos.positionType;
	if (positionType) style.position = positionType;
	if (pos.top) style.top = pos.top;
	if (pos.right) style.right = pos.right;
	if (pos.bottom) style.bottom = pos.bottom;
	if (pos.left) style.left = pos.left;
	return style;
}

function transformToCss(transform: Transform): React.CSSProperties {
	const transforms: string[] = [];
	if (transform.translate) transforms.push(`translate(${transform.translate})`);
	if (transform.rotate !== undefined)
		transforms.push(`rotate(${transform.rotate}deg)`);
	if (transform.scale) transforms.push(`scale(${transform.scale})`);
	if (transform.skew) transforms.push(`skew(${transform.skew})`);

	const style: React.CSSProperties = {};
	if (transforms.length > 0) style.transform = transforms.join(" ");
	if (transform.transformOrigin)
		style.transformOrigin = transform.transformOrigin;
	return style;
}

function spacingToCss(
	spacing: Spacing,
	type: "margin" | "padding",
): React.CSSProperties {
	if ("value" in spacing) {
		return { [type]: spacing.value };
	}

	const style: React.CSSProperties = {};
	if (spacing.top) style[`${type}Top`] = spacing.top;
	if (spacing.right) style[`${type}Right`] = spacing.right;
	if (spacing.bottom) style[`${type}Bottom`] = spacing.bottom;
	if (spacing.left) style[`${type}Left`] = spacing.left;
	return style;
}

function styleValueToCss(value: StyleValue): string {
	return typeof value === "string" ? value : value.value;
}

function spacingValueToCss(value: Spacing): string | undefined {
	if ("value" in value) return value.value;
	const sides = [value.top, value.right, value.bottom, value.left];
	return sides.some((side) => side !== undefined)
		? sides.map((side) => side ?? "0").join(" ")
		: undefined;
}

function toKebabCase(value: string): string {
	return value.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

function responsiveToCssVariables(
	overrides: ResponsiveOverrides,
): React.CSSProperties {
	const variables: Record<string, string | number> = {};
	const breakpoints = ["sm", "md", "lg", "xl", "xxl"] as const;

	for (const breakpoint of breakpoints) {
		const value = overrides[breakpoint];
		if (!value) continue;
		const set = (
			property: string,
			propertyValue: string | number | undefined,
		) => {
			if (propertyValue !== undefined) {
				variables[`--a2ui-${breakpoint}-${property}`] = propertyValue;
			}
		};

		set("display", value.hidden ? "none" : value.display);
		set("flex-direction", value.flexDirection);
		set("justify-content", value.justifyContent);
		set("align-items", value.alignItems);
		set("gap", value.gap);
		set(
			"grid-cols",
			value.gridCols === undefined
				? undefined
				: `repeat(${value.gridCols}, minmax(0, 1fr))`,
		);
		set("width", value.width && styleValueToCss(value.width));
		set("height", value.height && styleValueToCss(value.height));
		set("padding", value.padding && spacingValueToCss(value.padding));
		set("margin", value.margin && spacingValueToCss(value.margin));
		set("font-size", value.fontSize);
		set("text-align", value.textAlign);
		set("order", value.order);
	}

	return variables as React.CSSProperties;
}

function gradientStopPositionToCss(
	position: number | undefined,
	isLegacyRustShape: boolean,
): string | undefined {
	if (position === undefined) return undefined;
	return `${isLegacyRustShape && position >= 0 && position <= 1 ? position * 100 : position}%`;
}

// Utility to merge multiple styles
export function mergeStyles(...styles: (Style | undefined)[]): {
	className: string;
	style: React.CSSProperties;
} {
	const classNames: string[] = [];
	const inlineStyles: React.CSSProperties[] = [];

	for (const s of styles) {
		if (s) {
			classNames.push(resolveStyle(s));
			inlineStyles.push(resolveInlineStyle(s));
		}
	}

	return {
		className: cn(...classNames),
		style: Object.assign({}, ...inlineStyles),
	};
}
