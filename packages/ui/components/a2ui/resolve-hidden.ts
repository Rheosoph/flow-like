import type { BoundValue } from "./types";

/**
 * Resolve a component's `hidden` property. The value reaches the renderer as a BoundValue, and a
 * binding fed from a flow may deliver a string ("true"/"false") when the producing pin is typed as
 * text, so both shapes are accepted.
 *
 * Shared by every render path — surface components (A2UIRenderer) and widget-internal children
 * (A2UIWidgetInstance) — so a `hidden` binding behaves the same inside a widget as outside one.
 */
export function resolveHidden(
	hidden: unknown,
	resolve: (boundValue: BoundValue, defaultValue?: unknown) => unknown,
): boolean {
	if (hidden === undefined) return false;
	const value = resolve(hidden as BoundValue, false);
	return value === true || value === "true";
}
