import postcss, { type Root } from "postcss";
import type { CSSProperties } from "react";
import { cn } from "../../lib/utils";
import type { IHomeWidget } from "./types";

export const HOME_ACCENTS: Record<string, string> = {
	neutral: "var(--foreground)",
	violet: "#a78bfa",
	blue: "#60a5fa",
	emerald: "#34d399",
	orange: "#fb713f",
	amber: "#fbbf24",
	rose: "#fb7185",
};

export function homeAppearanceStyle(
	appearance: IHomeWidget["appearance"],
): CSSProperties {
	const accent = HOME_ACCENTS[appearance.accent] ?? HOME_ACCENTS.neutral;
	const accentForeground =
		accent === HOME_ACCENTS.neutral ? "var(--background)" : "#16131d";
	const solid = appearance.variant === "solid";
	const foreground = solid ? accentForeground : "var(--foreground)";
	return {
		"--home-accent": accent,
		"--home-accent-foreground": accentForeground,
		"--home-surface-foreground": foreground,
		"--home-surface-background": solid
			? accent
			: appearance.variant === "tinted"
				? `color-mix(in srgb, ${accent} 9%, var(--card))`
				: "var(--card)",
		"--home-surface-muted": solid
			? `color-mix(in srgb, ${foreground} 75%, transparent)`
			: "var(--muted-foreground)",
		"--home-surface-accent": solid
			? foreground
			: `color-mix(in srgb, ${accent} 65%, var(--foreground))`,
		"--home-surface-item": solid
			? `color-mix(in srgb, ${foreground} 8%, transparent)`
			: "color-mix(in srgb, var(--muted) 40%, transparent)",
		"--home-surface-item-hover": solid
			? `color-mix(in srgb, ${foreground} 14%, transparent)`
			: "var(--muted)",
		"--home-surface-border": solid
			? `color-mix(in srgb, ${foreground} 18%, transparent)`
			: "var(--border)",
	} as CSSProperties;
}

export function homeWidgetCssScope(id: string) {
	const value = id
		.replace(/["\\]/g, "\\$&")
		.replace(/[\n\r\f]/g, (char) => `\\${char.charCodeAt(0).toString(16)} `);
	return `[data-home-widget-style="${value}"]`;
}

const KEYFRAME_NAME = /^-?[_a-zA-Z][\w-]*$/;

function keyframeSuffix(id: string) {
	let hash = 0x811c9dc5;
	for (let index = 0; index < id.length; index++) {
		hash ^= id.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193);
	}
	return `hw${(hash >>> 0).toString(36)}`;
}

/** Keyframe names are document-global, so a widget's own names get a per-widget suffix. */
export function localizeHomeWidgetKeyframes(css: string, id: string) {
	let root: Root;
	try {
		root = postcss.parse(css);
	} catch {
		return css;
	}
	const names = new Map<string, string>();
	const suffix = keyframeSuffix(id);
	root.walkAtRules(/^(?:-webkit-)?keyframes$/i, (rule) => {
		const name = rule.params.trim();
		if (!KEYFRAME_NAME.test(name)) return;
		const local = names.get(name) ?? `${name}-${suffix}`;
		names.set(name, local);
		rule.params = local;
	});
	if (names.size === 0) return css;
	root.walkDecls(/^(?:-webkit-)?animation(?:-name)?$/i, (declaration) => {
		declaration.value = declaration.value.replace(
			/[^\s,()]+/g,
			(token) => names.get(token) ?? token,
		);
	});
	return root.toString();
}

// Important keeps editor feedback above the widget's own CSS, which is unlayered.
const HOME_SELECTED_CLASSES =
	"ring-2! ring-primary! ring-offset-2! ring-offset-background!";
const HOME_PLACEHOLDER_CLASSES =
	"border-dashed! border-primary/70! ring-1! ring-primary/25!";
const HOME_PLACEHOLDER_FILL_CLASSES = "bg-primary/5!";

export const HOME_PLACEHOLDER_CLASS_LIST = [
	HOME_PLACEHOLDER_CLASSES,
	HOME_PLACEHOLDER_FILL_CLASSES,
]
	.join(" ")
	.split(" ");

/** Classes for the styled surface inside the widget's shell; surface colors stay in classes so the widget's own Tailwind classes can replace them. */
export function homeWidgetFrameClassName({
	appearance,
	editing,
	selected,
	placeholder,
	autoHeight,
}: {
	appearance: IHomeWidget["appearance"];
	editing: boolean;
	selected: boolean;
	placeholder: boolean;
	autoHeight: boolean;
}) {
	const borderless = appearance.variant === "borderless";
	const filled =
		appearance.variant === "tinted" || appearance.variant === "solid";
	return cn(
		"relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-2xl text-[var(--home-surface-foreground)]",
		borderless && !editing && autoHeight && "overflow-visible rounded-none",
		borderless
			? "bg-transparent"
			: "border border-border/60 bg-card/70 shadow-sm",
		editing && "border border-border/70 bg-card/80",
		filled && "bg-[var(--home-surface-background)]",
		appearance.className,
		selected && editing && HOME_SELECTED_CLASSES,
		placeholder && HOME_PLACEHOLDER_CLASSES,
		placeholder && !filled && HOME_PLACEHOLDER_FILL_CLASSES,
	);
}
