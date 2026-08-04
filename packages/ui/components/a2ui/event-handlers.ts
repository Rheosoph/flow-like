import type { Action, EventHandlers } from "./types";

export const WILDCARD_EVENT = "*";

export interface EventActionResolution {
	actions: Action[];
	source: "event" | "wildcard" | "legacy" | "none";
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
 */
export function resolveEventActions(
	eventHandlers: EventHandlers | undefined,
	eventName: string,
	legacyActions: Action[] | undefined,
	legacyFallback = true,
): EventActionResolution {
	if (ownsEvent(eventHandlers, eventName)) {
		return {
			actions: eventHandlers?.[eventName] ?? [],
			source: "event",
		};
	}

	if (ownsEvent(eventHandlers, WILDCARD_EVENT)) {
		return {
			actions: eventHandlers?.[WILDCARD_EVENT] ?? [],
			source: "wildcard",
		};
	}

	if (legacyFallback && legacyActions?.[0]) {
		return { actions: [legacyActions[0]], source: "legacy" };
	}

	return { actions: [], source: "none" };
}

export function firstEventAction(
	eventHandlers: EventHandlers | undefined,
	eventName: string,
	legacyActions: Action[] | undefined,
	legacyFallback = true,
): Action | undefined {
	return resolveEventActions(
		eventHandlers,
		eventName,
		legacyActions,
		legacyFallback,
	).actions[0];
}
