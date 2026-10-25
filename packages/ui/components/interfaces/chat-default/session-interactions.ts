import { useSyncExternalStore } from "react";
import type { IInteractionRequest } from "../../../lib/schema/interaction";

const EMPTY: IInteractionRequest[] = [];
let bySession: ReadonlyMap<string, IInteractionRequest[]> = new Map();
const listeners = new Set<() => void>();

const subscribe = (listener: () => void) => {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
};
const snapshot = () => bySession;

/**
 * Interaction requests per chat session, held outside the chat component. Answers never travel
 * through the run's event stream, so a remounted chat replays every request as pending; only this
 * record knows which ones were already answered.
 */
export function updateSessionInteractions(
	sessionId: string,
	update: (prev: IInteractionRequest[]) => IInteractionRequest[],
): void {
	const prev = bySession.get(sessionId) ?? EMPTY;
	const next = update(prev);
	if (next === prev) return;
	const map = new Map(bySession);
	map.set(sessionId, next);
	bySession = map;
	for (const listener of listeners) listener();
}

/** Adds new requests; a known one only moves on from pending, never back to it. */
export function addSessionInteractions(
	sessionId: string,
	interactions: IInteractionRequest[],
): void {
	updateSessionInteractions(sessionId, (prev) => {
		const byId = new Map(
			prev.map((interaction) => [interaction.id, interaction]),
		);
		let changed = false;
		for (const interaction of interactions) {
			const existing = byId.get(interaction.id);
			if (
				!existing ||
				(existing.status === "pending" && interaction.status !== "pending")
			) {
				byId.set(interaction.id, interaction);
				changed = true;
			}
		}
		return changed ? Array.from(byId.values()) : prev;
	});
}

export function useSessionInteractions(
	sessionId: string,
): IInteractionRequest[] {
	return (
		useSyncExternalStore(subscribe, snapshot, snapshot).get(sessionId) ?? EMPTY
	);
}
