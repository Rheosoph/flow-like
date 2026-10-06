import {
	BUSY_RUN_STATUSES,
	type ExecutionTarget,
	FORM_LIMITS,
	type FailureLine,
	type HoldInfo,
	type RunEntry,
	type RunStatus,
} from "../contracts";

/** What the queue reads of a run. */
export type QueueRun = Pick<
	RunEntry,
	"id" | "n" | "status" | "createdAt" | "waitingForPlace"
>;

/**
 * How many runs this window keeps going at once (spec M5, `flpCap`): on this device
 * FORM_LIMITS.localParallel; in the cloud the tier's `max_concurrent_executions` (−1: no cap), and
 * FORM_LIMITS.unknownTierParallel until the tier is known. An unresolved target counts as the cloud.
 */
export function capOf(
	target: ExecutionTarget | null,
	tierLimit: number | null,
) {
	if (target === "local") return FORM_LIMITS.localParallel;
	if (tierLimit === null || !Number.isFinite(tierLimit))
		return FORM_LIMITS.unknownTierParallel;
	return tierLimit < 0 ? -1 : Math.floor(tierLimit);
}

const isBusy = (status: RunStatus) => BUSY_RUN_STATUSES.includes(status);

function inLineBefore(a: QueueRun, b: QueueRun) {
	if (a.waitingForPlace !== b.waitingForPlace)
		return a.waitingForPlace ? -1 : 1;
	return a.createdAt - b.createdAt || a.n - b.n;
}

/**
 * Queued runs in the order they will start: a run refused for want of a place goes back to the front, the
 * rest first in, first out by press time. Sending runs are not in line.
 */
function line(runs: readonly QueueRun[]) {
	return runs.filter((run) => run.status === "queued").sort(inLineBefore);
}

/**
 * Which queued runs start now (spec M5, `flpStartable`): starting, asking, running and streaming runs hold a
 * place; sending runs hold none. `cap` −1: no cap. `held`: the hold after two failures, or a refused start
 * waiting for its retry (`queue.retryAt`): nothing starts. A run asking the person a question holds its place
 * and never holds the queue. Any run order.
 */
export function startableRunIds(
	runsOldestFirst: readonly QueueRun[],
	cap: number,
	held: boolean,
) {
	if (held) return [];
	const busy = runsOldestFirst.filter((run) => isBusy(run.status)).length;
	const free = cap < 0 ? Number.POSITIVE_INFINITY : Math.max(0, cap - busy);
	return line(runsOldestFirst)
		.slice(0, free)
		.map((run) => run.id);
}

/** Places in line of the queued runs, by run id: `{ [id]: 1, … }` ("1st in line"). Any run order. */
export const linePlaces = (
	runs: readonly QueueRun[],
): Readonly<Record<string, number>> =>
	Object.fromEntries(line(runs).map((run, index) => [run.id, index + 1]));

/** What the hold reads of a run. */
export type HoldRun = Pick<
	RunEntry,
	"n" | "status" | "failedAt" | "endedAt" | "origin"
>;

const RAN_TO_AN_END: readonly RunStatus[] = [
	"done",
	"empty",
	"failed",
	"stopped",
];

const ranToAnEnd = (run: HoldRun) =>
	run.origin === "session" &&
	run.endedAt !== null &&
	RAN_TO_AN_END.includes(run.status);

function failedAtSameStep(a: HoldRun, b: HoldRun) {
	if (a.status !== "failed" || b.status !== "failed") return false;
	if (!a.failedAt || !b.failedAt) return false;
	return (
		a.failedAt.number === b.failedAt.number &&
		a.failedAt.title === b.failedAt.title
	);
}

/**
 * The hold (spec M5, `flpHold`): the last two runs of this session that ran to an end both failed at the same
 * step (number and title). Runs that never started (taken out of the queue) do not break the series; a done,
 * empty or stopped run does. Any run order: runs are put in end order here.
 */
export const holdOf = (
	endedOldestFirst: readonly HoldRun[],
): HoldInfo | null => {
	const ended = endedOldestFirst
		.filter(ranToAnEnd)
		.sort((a, b) => (a.endedAt ?? 0) - (b.endedAt ?? 0));
	if (ended.length < 2) return null;
	const [a, b] = ended.slice(-2);
	if (!failedAtSameStep(a, b) || !b.failedAt) return null;
	return { runs: [a.n, b.n], step: b.failedAt };
};

/** Failed runs the person has not picked yet, in the order given (state.runs: newest first). */
export const unseenFailures = (
	runs: readonly Pick<
		RunEntry,
		"id" | "n" | "status" | "unseenFailure" | "failedAt"
	>[],
): FailureLine[] =>
	runs
		.filter((run) => run.status === "failed" && run.unseenFailure)
		.map((run) => ({ runId: run.id, n: run.n, step: run.failedAt }));
