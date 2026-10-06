/*
 * Runs of this session: a new run with its own copy, starting it, the queue (spec M5) and the stage
 * following the newest started run. Shared by the press, Run again and the queue's inputs.
 */
import {
	type CopyValue,
	FORM_LIMITS,
	type FormSessionState,
	type NotStartedReason,
	type RunCopy,
	type RunEntry,
} from "../contracts";
import { startableRunIds } from "../model/queue";
import { sameValues } from "../model/values";
import { EMPTY_RUN_SUMMARY } from "../run/summary";
import {
	type Tx,
	changeRun,
	emit,
	nextSeq,
	persistRun,
	withPrefs,
	withRail,
	withView,
} from "./reduce-tx";
import { pendingSlotIds } from "./slots";
import { busyCount, newestSessionRun, runById } from "./state";

/** Unique per window and form: the display number, the moment and this window's sequence number. */
function runIdOf(n: number, tx: Tx) {
	return `run-${n}-${tx.clock.now.toString(36)}-${nextSeq(tx).toString(36)}`;
}

/**
 * A run of this session with its copy: `sending` while its files upload, else `queued` (the queue
 * starts it when a place is free). Takes the next display number and saves both.
 */
export function addRun(
	state: FormSessionState,
	copy: RunCopy,
	tx: Tx,
): { readonly state: FormSessionState; readonly run: RunEntry } {
	const n = Math.max(1, state.memory.prefs.nextRunNumber);
	const pending = pendingSlotIds(state.form.fields, copy.values);
	const run: RunEntry = {
		id: runIdOf(n, tx),
		n,
		origin: "session",
		status: pending.length > 0 ? "sending" : "queued",
		createdAt: tx.clock.now,
		startedAt: null,
		endedAt: null,
		copy,
		target: state.queue.target,
		streamId: null,
		backendRunId: null,
		output: null,
		outcome: null,
		summary: EMPTY_RUN_SUMMARY,
		stopRequested: false,
		pendingSlotIds: pending,
		waitingForPlace: false,
		failedAt: null,
		unseenFailure: false,
	};
	const added = withPrefs({ ...state, runs: [run, ...state.runs] }, tx, {
		nextRunNumber: n + 1,
	});
	persistRun(added, tx, run);
	return { state: added, run };
}

/** A stream id per dispatch: a run refused for want of a place is dispatched again on a new stream. */
function streamIdOf(run: RunEntry) {
	const attempt = run.streamId
		? Number(run.streamId.slice(run.streamId.lastIndexOf("#") + 1)) + 1
		: 1;
	return `${run.id}#${Number.isFinite(attempt) ? attempt : 1}`;
}

/** The run on the stage, compared with the rail (a run of this session, or one the person picked). */
export function showOnStage(
	state: FormSessionState,
	id: string,
): FormSessionState {
	return withRail(withView(state, { selectedRunId: id }), {
		comparedRunId: id,
	});
}

/** Dispatches a run; it takes the stage when the stage follows the newest started run. */
export function startRun(
	state: FormSessionState,
	id: string,
	tx: Tx,
): FormSessionState {
	const run = runById(state, id);
	if (!run) return state;
	const started = changeRun(state, tx, id, (entry) => ({
		...entry,
		status: "starting",
		target: entry.target ?? state.queue.target,
		streamId: streamIdOf(entry),
		waitingForPlace: false,
		pendingSlotIds: [],
	}));
	emit(tx, { type: "dispatchRun", runId: id });
	return started.view.stageFollowsNewest ? showOnStage(started, id) : started;
}

/**
 * Starts queued runs while places are free (`startableRunIds`): nothing starts while the queue is on
 * hold or a refused start waits for its retry, so the queue stays first in, first out.
 */
export function startQueued(state: FormSessionState, tx: Tx): FormSessionState {
	const held = state.queue.hold !== null || state.queue.retryAt !== null;
	const ids = startableRunIds(state.runs, state.queue.cap, held);
	return ids.reduce((current, id) => startRun(current, id, tx), state);
}

/** A run that never started: taken out of the queue, its file not sent, or the form left before its turn. */
export function notStarted(
	state: FormSessionState,
	tx: Tx,
	id: string,
	reason: NotStartedReason,
): FormSessionState {
	const next = changeRun(state, tx, id, (run) => ({
		...run,
		status: "notStarted",
		outcome: { kind: "notStarted", reason },
		endedAt: tx.clock.now,
		pendingSlotIds: [],
		waitingForPlace: false,
	}));
	return withoutStaleRetry(next);
}

/** A refused start waits for its retry only while some run still waits for a free place. */
export function withoutStaleRetry(state: FormSessionState): FormSessionState {
	if (state.queue.retryAt === null) return state;
	const waiting = state.runs.some(
		(run) => run.status === "queued" && run.waitingForPlace,
	);
	return waiting
		? state
		: { ...state, queue: { ...state.queue, retryAt: null } };
}

/**
 * A's 700 ms rule for every press (spec M2): within FORM_LIMITS.doublePressMs of the newest run of
 * this window, whatever its state, the same inputs start nothing.
 */
export function isDoublePress(
	state: FormSessionState,
	values: Readonly<Record<string, CopyValue>>,
	now: number,
): boolean {
	const newest = newestSessionRun(state);
	if (!newest || now - newest.createdAt >= FORM_LIMITS.doublePressMs)
		return false;
	return sameValues(state.form.fields, values, newest.copy.values);
}

/** A form without fields at its cap: the press is refused, never queued (spec M5). */
export function atCap(state: FormSessionState) {
	const { cap } = state.queue;
	return cap >= 0 && busyCount(state) >= cap;
}

/** "3 runs are going. You can run again when one ends." in the live region; focus stays. */
export function refuseAtCap(state: FormSessionState, tx: Tx): FormSessionState {
	return {
		...state,
		announcement: {
			seq: nextSeq(tx),
			kind: "capReached",
			running: busyCount(state),
		},
	};
}
