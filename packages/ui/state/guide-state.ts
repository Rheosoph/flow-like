import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import {
	type GuideSignal,
	type GuideTourId,
	type IGuideRun,
	applyLocation,
	applySignal,
	isRunFinished,
	skipStep,
	startRun,
} from "../lib/guide/guide-model";
import { GUIDE_TOURS } from "../lib/guide/guide-tours";

interface GuideState {
	/**
	 * "unknown" until the provider decides whether this is a new user; "pending"
	 * shows the welcome question; "done" never shows it again on its own.
	 */
	welcome: "unknown" | "pending" | "done";
	role: GuideTourId | null;
	run: IGuideRun | null;
	/** The tour that just ended by being completed, for the closing card. */
	finished: GuideTourId | null;
	completed: GuideTourId[];
	requestWelcome: () => void;
	decideWelcome: (show: boolean) => void;
	answerWelcome: (tour: GuideTourId | null) => void;
	startTour: (tour: GuideTourId) => void;
	signal: (signal: GuideSignal) => void;
	locate: (path: string) => void;
	skip: () => void;
	endTour: () => void;
	closeFinished: () => void;
}

function settle(
	state: Pick<GuideState, "run" | "completed">,
	run: IGuideRun,
): Partial<GuideState> {
	if (run === state.run) return {};
	const tour = GUIDE_TOURS[run.tour];
	if (!isRunFinished(tour, run)) return { run };
	return {
		run: null,
		finished: run.tour,
		completed: state.completed.includes(run.tour)
			? state.completed
			: [...state.completed, run.tour],
	};
}

export const useGuideStore = create<GuideState>()(
	persist(
		(set) => ({
			welcome: "unknown",
			role: null,
			run: null,
			finished: null,
			completed: [],
			requestWelcome: () => set({ welcome: "pending" }),
			decideWelcome: (show) =>
				set((state) =>
					state.welcome === "unknown"
						? { welcome: show ? "pending" : "done" }
						: {},
				),
			answerWelcome: (tour) =>
				set({
					welcome: "done",
					role: tour,
					run: tour ? startRun(GUIDE_TOURS[tour]) : null,
					finished: null,
				}),
			startTour: (tour) =>
				set({
					welcome: "done",
					run: startRun(GUIDE_TOURS[tour]),
					finished: null,
				}),
			signal: (signal) =>
				set((state) =>
					state.run
						? settle(
								state,
								applySignal(GUIDE_TOURS[state.run.tour], state.run, signal),
							)
						: {},
				),
			locate: (path) =>
				set((state) =>
					state.run
						? settle(
								state,
								applyLocation(GUIDE_TOURS[state.run.tour], state.run, path),
							)
						: {},
				),
			skip: () =>
				set((state) =>
					state.run
						? settle(state, skipStep(GUIDE_TOURS[state.run.tour], state.run))
						: {},
				),
			endTour: () => set({ run: null, finished: null }),
			closeFinished: () => set({ finished: null }),
		}),
		{
			name: "guide-storage",
			storage: createJSONStorage(() => localStorage),
			partialize: (state) => ({
				welcome: state.welcome,
				role: state.role,
				run: state.run,
				finished: state.finished,
				completed: state.completed,
			}),
		},
	),
);
