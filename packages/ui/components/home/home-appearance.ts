import type { CSSProperties } from "react";
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
		backgroundColor: solid
			? accent
			: appearance.variant === "tinted"
				? `color-mix(in srgb, ${accent} 9%, var(--card))`
				: undefined,
		color: foreground,
	} as CSSProperties;
}
