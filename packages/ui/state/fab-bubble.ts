import { useEffect } from "react";
import { create } from "zustand";

interface FabBubbleState {
	requests: number;
	suppressions: number;
	/** Increments the request count and returns a release function. */
	acquireRequest: () => () => void;
	/** Increments the suppression count and returns a release function. */
	acquireSuppression: () => () => void;
}

/**
 * Ref-counted visibility for the global FlowPilot bubble launcher.
 *
 * The launcher is opt-in: it only appears on surfaces that ask for it, because those are the
 * surfaces that delegate their assistant to it (the board and widget builders pass
 * `externalAssistant`, so they mount no FlowPilot button of their own) or that FlowPilot can act
 * on directly (Data Studio). Everywhere else it just floats over the UI, so it stays hidden.
 *
 * Suppression outranks a request: components that occupy the bottom-right corner (e.g. the
 * FlowScript panel, whose Apply button sits exactly where the bubble floats) hide it while mounted
 * even on a requesting surface. The bubble returns once every suppressor has released.
 */
export const useFabBubbleStore = create<FabBubbleState>((set) => ({
	requests: 0,
	suppressions: 0,
	acquireRequest: () => {
		set((state) => ({ requests: state.requests + 1 }));
		return () =>
			set((state) => ({ requests: Math.max(0, state.requests - 1) }));
	},
	acquireSuppression: () => {
		set((state) => ({ suppressions: state.suppressions + 1 }));
		return () =>
			set((state) => ({ suppressions: Math.max(0, state.suppressions - 1) }));
	},
}));

/** Show the global FlowPilot bubble launcher while the calling component is mounted and `active`. */
export function useRequestFabBubble(active = true): void {
	useEffect(() => {
		if (!active) return;
		return useFabBubbleStore.getState().acquireRequest();
	}, [active]);
}

/** Hide the global FlowPilot bubble launcher while the calling component is mounted and `active`. */
export function useSuppressFabBubble(active = true): void {
	useEffect(() => {
		if (!active) return;
		return useFabBubbleStore.getState().acquireSuppression();
	}, [active]);
}

/** True while some surface wants the launcher and nothing is claiming the bottom-right corner. */
export function useFabBubbleVisible(): boolean {
	return useFabBubbleStore(
		(state) => state.requests > 0 && state.suppressions === 0,
	);
}
