import type { ThemeMode, ThemeState } from "./protocol";
import { DEFAULT_DARK_TOKENS, DEFAULT_LIGHT_TOKENS } from "./theme";

export function themeForMode(mode: ThemeMode): ThemeState {
	return {
		mode,
		tokens: mode === "dark" ? DEFAULT_DARK_TOKENS : DEFAULT_LIGHT_TOKENS,
	};
}

export function detectColorScheme(): ThemeMode {
	if (typeof window === "undefined" || typeof window.matchMedia !== "function")
		return "light";
	return window.matchMedia("(prefers-color-scheme: dark)").matches
		? "dark"
		: "light";
}

export function watchColorScheme(
	onChange: (mode: ThemeMode) => void,
): () => void {
	if (typeof window === "undefined" || typeof window.matchMedia !== "function")
		return () => {};
	const query = window.matchMedia("(prefers-color-scheme: dark)");
	const listener = (event: MediaQueryListEvent) => {
		onChange(event.matches ? "dark" : "light");
	};
	query.addEventListener("change", listener);
	return () => query.removeEventListener("change", listener);
}

export function renderStandaloneBadge(): () => void {
	if (typeof document === "undefined" || !document.body) return () => {};
	const badge = document.createElement("div");
	badge.textContent = "standalone";
	badge.setAttribute(
		"style",
		[
			"position: fixed",
			"bottom: 8px",
			"right: 8px",
			"z-index: 2147483647",
			"padding: 2px 8px",
			"border-radius: 9999px",
			"font: 11px/1.6 ui-monospace, monospace",
			"color: rgba(255, 255, 255, 0.85)",
			"background: rgba(0, 0, 0, 0.55)",
			"pointer-events: none",
			"user-select: none",
		].join("; "),
	);
	document.body.appendChild(badge);
	return () => badge.remove();
}
