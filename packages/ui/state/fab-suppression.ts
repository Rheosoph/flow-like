import { useEffect } from "react";
import { create } from "zustand";

interface FabSuppressionState {
	count: number;
	/** Increments the suppression count and returns a release function. */
	acquire: () => () => void;
}

/**
 * Ref-counted suppression for the global FlowPilot bubble launcher. Components that occupy the
 * bottom-right corner (e.g. the FlowScript panel, whose Apply button sits exactly where the bubble
 * floats) suppress it while mounted so it doesn't overlap their actions; the bubble returns once
 * every suppressor has released.
 */
export const useFabSuppressionStore = create<FabSuppressionState>((set) => ({
	count: 0,
	acquire: () => {
		set((state) => ({ count: state.count + 1 }));
		return () => set((state) => ({ count: Math.max(0, state.count - 1) }));
	},
}));

/** Suppress the global FlowPilot bubble launcher for the lifetime of the calling component. */
export function useSuppressFabBubble(): void {
	useEffect(() => useFabSuppressionStore.getState().acquire(), []);
}

/** True while any mounted component is suppressing the bubble launcher. */
export function useFabBubbleSuppressed(): boolean {
	return useFabSuppressionStore((state) => state.count > 0);
}
