import type {
	DeployFailure,
	DeployOrder,
	DeployPhase,
	DeployResult,
} from "./deploy-plan";

/* The multi-device rollout as a pure state machine (APP §3.13, §7.8); `useDeployRun` drives the effects. */

export type DeployRowState =
	| "waiting"
	| "active"
	| "done"
	| "failed"
	| "blocked"
	| "skipped"
	| "held"
	| "not_started";

export interface DeployRunRow {
	/** One service on one device; the caller picks the key (`device/service`). */
	target: string;
	deviceId: string;
	serviceId?: string;
	phases: DeployPhase[];
	/** Current or failed phase; `phases.length` once done. */
	phase: number;
	state: DeployRowState;
	error?: DeployFailure;
	/** Why a blocked row waits, e.g. `locked`. */
	blocked?: string;
	progress?: { done: number; total: number };
	at?: number;
	startedAt?: number;
	finishedAt?: number;
}

/** Runs once before the device rows: the offline copy or the approved definitions. */
export type SharedPhase = "prepare" | "approve_definitions";

export interface DeployRunState {
	id: string;
	status: "idle" | "running" | "held" | "finished";
	order: DeployOrder;
	stopOnFail: boolean;
	/** The failed target that holds (or stopped) the rest. */
	holdBy: string | null;
	stopped: boolean;
	shared: {
		phase: SharedPhase;
		state: "waiting" | "active" | "done" | "failed";
		error?: DeployFailure;
	} | null;
	rows: DeployRunRow[];
	startedAt?: number;
	finishedAt?: number;
}

export type DeployRunEvent =
	| { type: "start"; at: number }
	| { type: "shared_done"; at: number }
	| { type: "shared_fail"; at: number; error: DeployFailure }
	| {
			type: "phase";
			target: string;
			phase: DeployPhase;
			at: number;
			progress?: { done: number; total: number };
	  }
	| { type: "done"; target: string; at: number }
	| { type: "fail"; target: string; at: number; error: DeployFailure }
	| { type: "block"; target: string; at: number; reason: string }
	| { type: "unblock"; target: string; at: number }
	| { type: "retry"; target: string; at: number }
	| { type: "skip"; target: string; at: number }
	| { type: "continue"; at: number }
	| { type: "stop"; at: number };

export function createDeployRun(input: {
	id: string;
	rows: Pick<DeployRunRow, "target" | "deviceId" | "serviceId" | "phases">[];
	order: DeployOrder;
	stopOnFail: boolean;
	shared?: SharedPhase | null;
}): DeployRunState {
	return {
		id: input.id,
		status: "idle",
		order: input.order,
		stopOnFail: input.stopOnFail,
		holdBy: null,
		stopped: false,
		shared: input.shared ? { phase: input.shared, state: "waiting" } : null,
		rows: input.rows.map((row) => ({ ...row, phase: 0, state: "waiting" })),
	};
}

const OPEN: readonly DeployRowState[] = [
	"waiting",
	"held",
	"active",
	"blocked",
];

const patchRow = (
	state: DeployRunState,
	target: string,
	patch: (row: DeployRunRow) => DeployRunRow,
): DeployRunState => {
	return {
		...state,
		rows: state.rows.map((row) => (row.target === target ? patch(row) : row)),
	};
};

const activate = (row: DeployRunRow, at: number): DeployRunRow => {
	return { ...row, state: "active", at, startedAt: row.startedAt ?? at };
};

const readyRows = (state: DeployRunState): Set<string> => {
	const waiting = state.rows.filter((row) => row.state === "waiting");
	if (state.order === "all") return new Set(waiting.map((row) => row.target));
	const busy = state.rows.some(
		(row) => row.state === "active" || row.state === "blocked",
	);
	if (busy) return new Set();
	const [first] = state.rows;
	const rest = state.order === "first" && first && first.state !== "waiting";
	return new Set(
		(rest ? waiting : waiting.slice(0, 1)).map((row) => row.target),
	);
};

/** Ends the run once nothing is open; a run that only has held rows left waits for the user's choice again. */
const finish = (state: DeployRunState, at: number): DeployRunState => {
	if (state.status === "idle") return state;
	if (!state.rows.some((row) => OPEN.includes(row.state)))
		return { ...state, status: "finished", finishedAt: at };
	const moving = state.rows.some(
		(row) => row.state === "active" || row.state === "blocked",
	);
	const held = state.rows.some((row) => row.state === "held");
	return held && !moving ? { ...state, status: "held" } : state;
};

/** Starts the rows whose turn it is (one at a time, all at once, or first then the rest). */
const schedule = (state: DeployRunState, at: number): DeployRunState => {
	const sharedPending = state.shared !== null && state.shared.state !== "done";
	if (state.status !== "running" || sharedPending) return finish(state, at);
	const ready = readyRows(state);
	return finish(
		{
			...state,
			rows: state.rows.map((row) =>
				ready.has(row.target) ? activate(row, at) : row,
			),
		},
		at,
	);
};

/** Lets held rows go on; without `force` only once no failure holds them. */
const release = (state: DeployRunState, force = false): DeployRunState => {
	const holding =
		state.stopOnFail && state.rows.some((row) => row.state === "failed");
	if (holding && !force) return state;
	return {
		...state,
		status: "running",
		holdBy: null,
		rows: state.rows.map((row) =>
			row.state === "held" ? { ...row, state: "waiting" } : row,
		),
	};
};

const onFail = (
	state: DeployRunState,
	event: Extract<DeployRunEvent, { type: "fail" }>,
): DeployRunState => {
	const failed = patchRow(state, event.target, (row) => ({
		...row,
		state: "failed",
		error: event.error,
		phase: Math.max(0, row.phases.indexOf(event.error.phase)),
		at: event.at,
		finishedAt: event.at,
	}));
	if (!state.stopOnFail) return schedule(failed, event.at);
	return finish(
		{
			...failed,
			status: "held",
			holdBy: event.target,
			rows: failed.rows.map((row) =>
				row.state === "waiting" ? { ...row, state: "held" } : row,
			),
		},
		event.at,
	);
};

const onRetry = (
	state: DeployRunState,
	target: string,
	at: number,
): DeployRunState => {
	const row = state.rows.find((value) => value.target === target);
	if (row?.state !== "failed") return state;
	const retried = patchRow(state, target, (value) => ({
		...activate(value, at),
		error: undefined,
		finishedAt: undefined,
	}));
	const running = {
		...retried,
		status: "running" as const,
		finishedAt: undefined,
	};
	return schedule(release(running), at);
};

const onSkip = (
	state: DeployRunState,
	target: string,
	at: number,
): DeployRunState => {
	const row = state.rows.find((value) => value.target === target);
	if (!row || !["failed", "blocked", "waiting", "held"].includes(row.state))
		return state;
	const skipped = patchRow(state, target, (value) => ({
		...value,
		state: "skipped",
		at,
		finishedAt: at,
	}));
	return schedule(skipped.status === "held" ? release(skipped) : skipped, at);
};

const onStop = (state: DeployRunState, at: number): DeployRunState => {
	return finish(
		{
			...state,
			status: "running",
			stopped: true,
			rows: state.rows.map((row) =>
				row.state === "waiting" || row.state === "held"
					? { ...row, state: "not_started", at }
					: row,
			),
		},
		at,
	);
};

const onPhase = (
	state: DeployRunState,
	event: Extract<DeployRunEvent, { type: "phase" }>,
): DeployRunState => {
	return patchRow(state, event.target, (row) => {
		const index = row.phases.indexOf(event.phase);
		if (row.state !== "active" || index < 0) return row;
		return {
			...row,
			phase: index,
			at: event.at,
			...(event.progress
				? { progress: event.progress }
				: { progress: undefined }),
		};
	});
};

const onDone = (
	state: DeployRunState,
	target: string,
	at: number,
): DeployRunState => {
	const row = state.rows.find((value) => value.target === target);
	if (row?.state !== "active") return state;
	return schedule(
		patchRow(state, target, (value) => ({
			...value,
			state: "done",
			phase: value.phases.length,
			progress: undefined,
			at,
			finishedAt: at,
		})),
		at,
	);
};

const onShared = (
	state: DeployRunState,
	event: Extract<DeployRunEvent, { type: "shared_done" | "shared_fail" }>,
): DeployRunState => {
	if (!state.shared || state.shared.state !== "active") return state;
	if (event.type === "shared_done")
		return schedule(
			{ ...state, shared: { ...state.shared, state: "done" } },
			event.at,
		);
	return onStop(
		{
			...state,
			shared: { ...state.shared, state: "failed", error: event.error },
		},
		event.at,
	);
};

const onStart = (state: DeployRunState, at: number): DeployRunState => {
	if (state.status !== "idle") return state;
	const started: DeployRunState = {
		...state,
		status: "running",
		startedAt: at,
		shared: state.shared ? { ...state.shared, state: "active" } : null,
	};
	return started.shared ? started : schedule(started, at);
};

const onBlock = (
	state: DeployRunState,
	event: Extract<DeployRunEvent, { type: "block" | "unblock" }>,
): DeployRunState => {
	const from = event.type === "block" ? "active" : "blocked";
	return patchRow(state, event.target, (row) =>
		row.state !== from
			? row
			: event.type === "block"
				? { ...row, state: "blocked", blocked: event.reason, at: event.at }
				: { ...row, state: "active", blocked: undefined, at: event.at },
	);
};

type Handlers = {
	[K in DeployRunEvent["type"]]: (
		state: DeployRunState,
		event: Extract<DeployRunEvent, { type: K }>,
	) => DeployRunState;
};

const HANDLERS: Handlers = {
	start: (state, event) => onStart(state, event.at),
	shared_done: onShared,
	shared_fail: onShared,
	phase: onPhase,
	done: (state, event) => onDone(state, event.target, event.at),
	fail: (state, event) =>
		state.rows.some(
			(row) => row.target === event.target && row.state === "active",
		)
			? onFail(state, event)
			: state,
	block: onBlock,
	unblock: onBlock,
	retry: (state, event) => onRetry(state, event.target, event.at),
	skip: (state, event) => onSkip(state, event.target, event.at),
	continue: (state, event) =>
		state.rows.some((row) => row.state === "held")
			? schedule(release(state, true), event.at)
			: state,
	stop: (state, event) =>
		state.status === "finished" ? state : onStop(state, event.at),
};

export function reduceDeployRun(
	state: DeployRunState,
	event: DeployRunEvent,
): DeployRunState {
	const handle = HANDLERS[event.type] as (
		state: DeployRunState,
		event: DeployRunEvent,
	) => DeployRunState;
	return handle(state, event);
}

/** Rows the effect runner should drive now. */
export function activeTargets(state: DeployRunState): string[] {
	return state.rows
		.filter((row) => row.state === "active")
		.map((row) => row.target);
}

/** The end result once nothing is left to run; null before that. */
export function deployRunResult(state: DeployRunState): DeployResult | null {
	if (state.status !== "finished") return null;
	const by = (wanted: DeployRowState) =>
		state.rows.filter((row) => row.state === wanted).map((row) => row.target);
	const done = by("done");
	return {
		outcome:
			done.length === state.rows.length
				? "all"
				: done.length
					? "partial"
					: "none",
		at: state.finishedAt ?? state.startedAt ?? 0,
		done,
		failed: state.rows.flatMap((row) =>
			row.state === "failed" && row.error
				? [{ ...row.error, target: row.target }]
				: [],
		),
		skipped: by("skipped"),
		notStarted: by("not_started"),
		...(state.shared?.error ? { sharedFailure: state.shared.error } : {}),
	};
}
