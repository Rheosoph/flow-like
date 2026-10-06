/*
 * What the run bar's lead line says, as descriptors (`copy.ts` turns them into words): the status,
 * how long a finished run took, the clock of a live one and the step, place in line or reason
 * after it. Spec M5 "Anatomy", canvas `paneVM`. Pure.
 */
import type {
	NotStartedReason,
	RunEntry,
	RunStatus,
	StepRef,
} from "../contracts";
import { elapsedMs } from "../run/run-view";
import { currentStepOf, failedStepOf, stoppedStepOf } from "./run-steps";
import { ticksClock } from "./strip-model";

export type BarLead =
	| { readonly kind: "status" }
	| { readonly kind: "doneIn"; readonly ms: number }
	| { readonly kind: "failedAfter"; readonly ms: number }
	| { readonly kind: "stoppedAt"; readonly ms: number };

export type BarDetail =
	| { readonly kind: "step"; readonly step: StepRef }
	| { readonly kind: "inLine"; readonly place: number }
	| { readonly kind: "waitingForPlace" }
	| { readonly kind: "sendingSoon"; readonly files: number }
	| { readonly kind: "notStarted"; readonly reason: NotStartedReason };

export interface BarView {
	readonly status: RunStatus;
	readonly lead: BarLead;
	/** The ticking clock after the lead (aria-hidden); null unless the run is live and has started. */
	readonly clockMs: number | null;
	readonly detail: BarDetail | null;
}

export interface BarInput {
	readonly now: number;
	/** `linePlaces(state.runs)`: a queued run's place by id. */
	readonly places: Readonly<Record<string, number>>;
}

const TOOK_LEAD: Readonly<Partial<Record<RunStatus, (ms: number) => BarLead>>> =
	{
		done: (ms) => ({ kind: "doneIn", ms }),
		empty: (ms) => ({ kind: "doneIn", ms }),
		failed: (ms) => ({ kind: "failedAfter", ms }),
		stopped: (ms) => ({ kind: "stoppedAt", ms }),
	};

const STATUS_LEAD: BarLead = { kind: "status" };

/** "Done in 48 s", "Failed after 12 s", "Stopped at 0:37"; a run that never started has only its word. */
function leadOf(run: RunEntry, now: number): BarLead {
	const lead = TOOK_LEAD[run.status];
	if (!lead || run.startedAt === null) return STATUS_LEAD;
	return lead(elapsedMs(run, now));
}

type DetailOf = (run: RunEntry, input: BarInput) => BarDetail | null;

const stepDetail = (step: StepRef | null): BarDetail | null =>
	step ? { kind: "step", step } : null;

const LIVE_DETAIL: DetailOf = (run) => stepDetail(currentStepOf(run));

const DETAIL: Readonly<Partial<Record<RunStatus, DetailOf>>> = {
	running: LIVE_DETAIL,
	streaming: LIVE_DETAIL,
	asking: LIVE_DETAIL,
	failed: (run) => stepDetail(failedStepOf(run)),
	stopped: (run) => stepDetail(stoppedStepOf(run)),
	sending: (run) => ({
		kind: "sendingSoon",
		files: Math.max(1, run.pendingSlotIds.length),
	}),
	queued: (run, input) => {
		const place = input.places[run.id];
		if (run.waitingForPlace || place === undefined)
			return run.waitingForPlace ? { kind: "waitingForPlace" } : null;
		return { kind: "inLine", place };
	},
	notStarted: (run) =>
		run.outcome?.kind === "notStarted"
			? { kind: "notStarted", reason: run.outcome.reason }
			: null,
};

export function barViewOf(run: RunEntry, input: BarInput): BarView {
	return {
		status: run.status,
		lead: leadOf(run, input.now),
		clockMs: ticksClock(run) ? elapsedMs(run, input.now) : null,
		detail: DETAIL[run.status]?.(run, input) ?? null,
	};
}
