import type { Action, EventHandlers } from "./types";

export const WILDCARD_EVENT = "*";

export interface EventActionResolution {
	actions: Action[];
	source: "event" | "wildcard" | "legacy" | "none";
}

export interface EventFallbackOptions {
	/** Allow the component's historical `actions[0]` to run this event. */
	legacyFallback?: boolean;
	/** Allow the `*` handler to run this event. */
	wildcardFallback?: boolean;
}

function ownsEvent(
	eventHandlers: EventHandlers | undefined,
	eventName: string,
): boolean {
	return Boolean(
		eventHandlers &&
			Object.prototype.hasOwnProperty.call(eventHandlers, eventName),
	);
}

/**
 * Resolve actions for a named component event without changing legacy behavior.
 *
 * Named and wildcard handlers are ordered action lists. An explicitly present
 * empty list disables that event. When neither is present, only `actions[0]`
 * is used because older runtimes intentionally ignored subsequent entries.
 *
 * Events added after a component shipped opt out of both fallbacks: a surface
 * authored before the event existed never meant to subscribe to it, and
 * high-frequency events such as `input` would otherwise fire a run per pause.
 */
export function resolveEventActions(
	eventHandlers: EventHandlers | undefined,
	eventName: string,
	legacyActions: Action[] | undefined,
	fallback: EventFallbackOptions = {},
): EventActionResolution {
	if (ownsEvent(eventHandlers, eventName)) {
		return {
			actions: eventHandlers?.[eventName] ?? [],
			source: "event",
		};
	}

	if (
		(fallback.wildcardFallback ?? true) &&
		ownsEvent(eventHandlers, WILDCARD_EVENT)
	) {
		return {
			actions: eventHandlers?.[WILDCARD_EVENT] ?? [],
			source: "wildcard",
		};
	}

	if ((fallback.legacyFallback ?? true) && legacyActions?.[0]) {
		return { actions: [legacyActions[0]], source: "legacy" };
	}

	return { actions: [], source: "none" };
}

export function firstEventAction(
	eventHandlers: EventHandlers | undefined,
	eventName: string,
	legacyActions: Action[] | undefined,
	fallback: EventFallbackOptions = {},
): Action | undefined {
	return resolveEventActions(eventHandlers, eventName, legacyActions, fallback)
		.actions[0];
}
