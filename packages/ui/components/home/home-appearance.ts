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

/** Surface colors stay in classes so the widget's own Tailwind classes can replace them. */
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
		"group/widget relative flex min-h-0 min-w-0 flex-col overflow-hidden rounded-2xl text-[var(--home-surface-foreground)]",
		borderless && !editing && autoHeight && "overflow-visible rounded-none",
		borderless
			? "bg-transparent"
			: "border border-border/60 bg-card/70 shadow-sm shadow-black/[0.02]",
		editing && "border border-border/70 bg-card/80",
		filled && "bg-[var(--home-surface-background)]",
		appearance.className,
		selected &&
			editing &&
			"ring-2 ring-primary ring-offset-2 ring-offset-background",
		placeholder && "border-dashed border-primary/70 ring-1 ring-primary/25",
		placeholder && !filled && "bg-primary/5",
	);
}
