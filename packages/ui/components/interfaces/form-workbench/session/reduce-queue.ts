/*
 * The queue and the run lifecycle (spec M5, PLAN §3.5): Stop on a live run asks the runtime to stop
 * it; on a queued or sending run it takes it out ("Not started"); "Clear queue" takes out everything
 * still waiting and the next files (Undo puts the files back); a start refused for want of a place
 * goes back to the front as "waiting for a free place" and is tried again when a run of this window
 * ends or after 15 s; two failures in a row at the same step hold the queue until "Resume". Each end
 * is announced; a failure stays unseen until the person picks the run.
 */
import {
	FORM_LIMITS,
	type FileSlot,
	type FormSessionState,
	LIVE_RUN_STATUSES,
	type RunEntry,
	type RunOutcome,
	type RunOutput,
	type RunStatus,
} from "../contracts";
import { capOf, holdOf } from "../model/queue";
import { isReminder, slotsOf } from "../model/values";
import { elapsedMs, liveStatusOf, statusOfOutcome } from "../run/run-view";
import { failedAtOf, markEndedSteps } from "../run/steps";
import { summaryOf } from "../run/summary";
import { endSeries } from "./reduce-per-run";
import { notStarted } from "./reduce-runs";
import {
	type CommandHandlers,
	type InputHandlers,
	type Tx,
	changeRun,
	clearEdits,
	emit,
	mapRun,
	nextSeq,
	withFocus,
	withRail,
	withUndoMessage,
	withView,
} from "./reduce-tx";
import { isBusy, isSplit, isWaiting, runById } from "./state";

const LIVE: ReadonlySet<RunStatus> = new Set(LIVE_RUN_STATUSES);

const sessionRun = (state: FormSessionState, id: string) => {
	const run = runById(state, id);
	return run?.origin === "session" ? run : null;
};

/** Stop (dock, run bar, ⌘.): a live run stops; a queued or sending run is taken out of the queue. */
const stop: CommandHandlers<"stop">["stop"] = (state, command, tx) => {
	const run = sessionRun(state, command.runId);
	if (!run) return state;
	if (isWaiting(run)) return notStarted(state, tx, run.id, "removedFromQueue");
	if (!isBusy(run) || run.stopRequested) return state;
	emit(tx, { type: "stopRun", runId: run.id });
	return mapRun(state, run.id, (entry) => ({ ...entry, stopRequested: true }));
};

/** The run bar's "Remove from queue": the run is taken out and the cursor goes to Run (PLAN §7 focus list). */
const removeFromQueue: CommandHandlers<"removeFromQueue">["removeFromQueue"] = (
	state,
	command,
	tx,
) => {
	const run = sessionRun(state, command.runId);
	if (!run || !isWaiting(run)) return state;
	return withFocus(notStarted(state, tx, run.id, "removedFromQueue"), tx, {
		kind: "run",
	});
};

const realCurrent = (value: unknown) =>
	slotsOf(value).some((slot: FileSlot) => !isReminder(slot));

/**
 * Next files all go; a field keeps its current file as the series' last. A field left without a file
 * of its series ends the series (the dock asks whether its fields stay per run).
 */
function withoutNextFiles(state: FormSessionState) {
	const { rail } = state;
	const names = Object.keys(rail.nextFiles);
	const kept = names
		.filter((name) => realCurrent(rail.values[name]))
		.map((name) => [name, []] as const);
	const list = state.view.list?.kind === "nextFiles" ? null : state.view.list;
	const cleared = withView(
		withRail(state, { nextFiles: Object.fromEntries(kept) }),
		{ list },
	);
	return kept.length < names.length ? endSeries(cleared) : cleared;
}

/** "Queue cleared. 1 run did not start and 5 next files were removed." with Undo for the files. */
const clearQueue: CommandHandlers<"clearQueue">["clearQueue"] = (
	state,
	_command,
	tx,
) => {
	const waiting = state.runs.filter(
		(run) => run.origin === "session" && isWaiting(run),
	);
	const files = Object.values(state.rail.nextFiles).reduce(
		(count, list) => count + list.length,
		0,
	);
	if (waiting.length === 0 && files === 0) return state;
	const taken = waiting.reduce(
		(current, run) => notStarted(current, tx, run.id, "removedFromQueue"),
		clearEdits(state),
	);
	const cleared = withoutNextFiles(taken);
	const released: FormSessionState = {
		...cleared,
		queue: { ...cleared.queue, hold: null, retryAt: null },
	};
	return withUndoMessage(
		released,
		tx,
		{ kind: "queueCleared", runs: waiting.length, files },
		{ kind: "queueCleared", before: state },
	);
};

const resumeQueue: CommandHandlers<"resumeQueue">["resumeQueue"] = (state) =>
	state.queue.hold === null
		? state
		: { ...state, queue: { ...state.queue, hold: null } };

export const QUEUE_COMMANDS: CommandHandlers<
	"stop" | "removeFromQueue" | "clearQueue" | "resumeQueue"
> = {
	stop,
	removeFromQueue,
	clearQueue,
	resumeQueue,
};

// ─── What the runtime reports ───────────────────────────────────────────────

/** `onExecutionStart`: the run is under way; its clock runs from here. */
const runAccepted: InputHandlers<"runAccepted">["runAccepted"] = (
	state,
	input,
	tx,
) => {
	const run = sessionRun(state, input.runId);
	if (!run || !LIVE.has(run.status)) return state;
	return changeRun(state, tx, run.id, (entry) => ({
		...entry,
		backendRunId: input.backendRunId,
		status: entry.status === "starting" ? "running" : entry.status,
		startedAt: entry.startedAt ?? tx.clock.now,
	}));
};

/** A live run's status from what it sent: asking > streaming > running; the first event starts its clock. */
function liveUpdate(
	run: RunEntry,
	output: RunOutput,
	state: FormSessionState,
	tx: Tx,
): RunEntry {
	const began = run.status !== "starting" || output.eventCount > 0;
	return {
		...run,
		output,
		status: began ? liveStatusOf(output, tx.clock.now) : run.status,
		startedAt: run.startedAt ?? (began ? tx.clock.now : null),
		summary: summaryOf(output, null, { locale: state.form.viewer.locale }),
	};
}

/** Saved when its start time changes (every live status is saved as `running`), never per chunk of text. */
const savedChanged = (before: RunEntry, after: RunEntry) =>
	before.startedAt !== after.startedAt;

const runOutput: InputHandlers<"runOutput">["runOutput"] = (
	state,
	input,
	tx,
) => {
	const run = sessionRun(state, input.runId);
	if (!run || isWaiting(run) || run.status === "notStarted") return state;
	if (!LIVE.has(run.status))
		return mapRun(state, run.id, (entry) => ({
			...entry,
			output: input.output,
		}));
	const updated = liveUpdate(run, input.output, state, tx);
	return savedChanged(run, updated)
		? changeRun(state, tx, run.id, () => updated)
		: mapRun(state, run.id, () => updated);
};

/** Refused for want of a place: back to the front of the queue, tried again later. */
function requeue(
	state: FormSessionState,
	run: RunEntry,
	tx: Tx,
): FormSessionState {
	const queued = changeRun(state, tx, run.id, (entry) => ({
		...entry,
		status: "queued",
		waitingForPlace: true,
		startedAt: null,
		backendRunId: null,
		output: null,
		outcome: null,
	}));
	if (queued.queue.retryAt !== null) return queued;
	emit(tx, { type: "scheduleRetry", afterMs: FORM_LIMITS.retryMs });
	return {
		...queued,
		queue: { ...queued.queue, retryAt: tx.clock.now + FORM_LIMITS.retryMs },
	};
}

type AnnouncedStatus = "done" | "empty" | "failed" | "stopped" | "unknown";

const ANNOUNCED: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"done",
	"empty",
	"failed",
	"stopped",
	"unknown",
]);

/**
 * "Run 15 done in 41 s.", "Run 18 failed after 12 s at step 2: Run OCR.", "Run 17 stopped at 0:37.",
 * "Run 16: the connection ended before it reported a result." A stop is read as the clock the stage
 * shows ("Stopped at 0:37", whole seconds never rounded up); a duration as "Done in 48 s" reads it
 * (rounded). Only session runs end here, so `unknown` is always a lost connection.
 */
function announceEnd(
	state: FormSessionState,
	run: RunEntry,
	tx: Tx,
): FormSessionState {
	if (!ANNOUNCED.has(run.status)) return state;
	const kind = run.status as AnnouncedStatus;
	const seconds = elapsedMs(run, tx.clock.now) / 1000;
	return {
		...state,
		announcement: {
			seq: nextSeq(tx),
			kind,
			n: run.n,
			seconds: kind === "stopped" ? Math.floor(seconds) : Math.round(seconds),
			step: run.status === "failed" ? run.failedAt : null,
		},
	};
}

/** Two failures in a row at the same step hold the queue until "Resume". */
function holdAfter(state: FormSessionState, run: RunEntry): FormSessionState {
	if (run.status !== "failed" || state.queue.hold !== null) return state;
	const hold = holdOf(state.runs);
	return hold ? { ...state, queue: { ...state.queue, hold } } : state;
}

/** The ended run as the stage and the lists show it. Picking it explicitly beforehand counts as seen. */
function endedEntry(
	state: FormSessionState,
	run: RunEntry,
	outcome: RunOutcome,
	tx: Tx,
): RunEntry {
	const output = run.output;
	const steps = output ? markEndedSteps(output.steps, outcome) : [];
	const status = statusOfOutcome(outcome, output);
	const { selectedRunId, stageFollowsNewest } = state.view;
	const picked = selectedRunId === run.id && !stageFollowsNewest;
	return {
		...run,
		status,
		outcome,
		endedAt: tx.clock.now,
		output: output ? { ...output, steps } : null,
		summary: summaryOf(output, outcome, { locale: state.form.viewer.locale }),
		failedAt: status === "failed" ? failedAtOf(steps) : null,
		unseenFailure: status === "failed" && !picked,
		pendingSlotIds: [],
	};
}

/** A run ended: its place is free, a waiting refused start is tried now, the end is announced. */
function endRun(
	state: FormSessionState,
	run: RunEntry,
	outcome: RunOutcome,
	tx: Tx,
): FormSessionState {
	const ended = endedEntry(state, run, outcome, tx);
	let next = changeRun(state, tx, run.id, () => ended);
	next = holdAfter(announceEnd(next, ended, tx), ended);
	next = { ...next, queue: { ...next.queue, retryAt: null } };
	if (!isSplit(next) && next.view.pane === "inputs")
		next = withView(next, { outputUnseen: true });
	return next;
}

const runSettled: InputHandlers<"runSettled">["runSettled"] = (
	state,
	input,
	tx,
) => {
	const run = sessionRun(state, input.runId);
	if (!run || !isBusy(run)) return state;
	if (input.outcome.kind === "noPlace") return requeue(state, run, tx);
	return endRun(state, run, input.outcome, tx);
};

/** The 15 s retry of a refused start; an older timer than the current retry is ignored. */
const retryDue: InputHandlers<"retryDue">["retryDue"] = (state, _input, tx) => {
	const { retryAt } = state.queue;
	if (retryAt === null || tx.clock.now < retryAt) return state;
	return { ...state, queue: { ...state.queue, retryAt: null } };
};

/** The app host's target and the tier's limit: the cap (spec M5). */
const targetResolved: InputHandlers<"targetResolved">["targetResolved"] = (
	state,
	input,
) => ({
	...state,
	queue: {
		...state.queue,
		target: input.target,
		tierLimit: input.tierLimit,
		cap: capOf(input.target, input.tierLimit),
	},
});

export const QUEUE_INPUTS: InputHandlers<
	"runAccepted" | "runOutput" | "runSettled" | "retryDue" | "targetResolved"
> = {
	runAccepted,
	runOutput,
	runSettled,
	retryDue,
	targetResolved,
};
