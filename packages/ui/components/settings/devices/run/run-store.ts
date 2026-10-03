"use client";

import {
	type EventRun,
	type RunRejection,
	cancelRun,
	readRun,
	runEvent,
	runOpen,
} from "../../../../lib/device-management/event-run";
import {
	ConnectError,
	ManagementRequestNotSentError,
	ManagementUnconfirmedError,
} from "../../../../lib/device-management/transport";
import type { ManagementRejection } from "../../../../lib/device-management/types";
import type { ActivityStart } from "../../../../lib/device-management/workspace/activity";
import {
	type DeviceFailure,
	LiveCallError,
	classifyDeviceError,
} from "../../../../lib/device-management/workspace/errors";
import type {
	ActivityItem,
	ActivityTarget,
	CallLane,
	DeviceWorkspace,
} from "../../../../lib/device-management/workspace/types";

/*
 * Runs of quick actions and forms that this page started or shows (design R2
 * §6.5). An open run is read until it ends, also after its sheet closed, and
 * its tray item follows it. Nothing here is written to storage: the tray item
 * holds ids, state and times; inputs and output stay in this page's memory.
 */

/** How often an open run is read: every `fastMs` for `fastForMs`, then every `slowMs`. Tests shorten it. */
export const runPace = {
	fastMs: 1_000,
	fastForMs: 10_000,
	slowMs: 3_000,
	/** Reads that fail in a row before reading stops until "Check again". */
	maxReadFailures: 5,
};

export type RunPhase =
	/** `run_event` is on its way. */
	| { kind: "sending" }
	/** A run started earlier is being looked up. */
	| { kind: "reading" }
	| { kind: "open"; run: EventRun }
	| { kind: "ended"; run: EventRun }
	/** Refused before anything was recorded on the device. */
	| {
			kind: "rejected";
			code: RunRejection | "other";
			rejection: ManagementRejection;
	  }
	/** It never reached the device: nothing ran. */
	| { kind: "not_sent"; failure: DeviceFailure }
	/** Sent, and no reply came: it may have started. */
	| { kind: "no_reply" }
	/** The device's records don't know it: it never arrived, or it is older than a day. */
	| { kind: "unknown" };

export interface RunEntry {
	operationId: string;
	deviceId: string;
	serviceId: string;
	eventId: string;
	phase: RunPhase;
	/** The last read failed; reading goes on. */
	readFailure?: DeviceFailure;
	/** Reading stopped (failures in a row, or the device was locked); "Check again" resumes it. */
	paused?: boolean;
	/** "Stop this run" was sent. */
	stopping?: boolean;
	stopFailure?: DeviceFailure;
	/** Unix milliseconds on this computer. */
	since: number;
}

export interface RunStart {
	deviceId: string;
	deviceName?: string;
	serviceId: string;
	projectId?: string;
	eventId: string;
	expectedRevision: number;
	payload?: Record<string, unknown>;
}

export interface RunLookup {
	operationId: string;
	deviceId: string;
	serviceId: string;
	eventId: string;
}

export interface RunStore {
	get(operationId: string): RunEntry | undefined;
	subscribe(listener: () => void): () => void;
	/** Sends one run; the operation id names it from now on. */
	start(start: RunStart): string;
	/** Shows a run started earlier: read from the device unless this page is reading it already. */
	view(lookup: RunLookup): void;
	/** "Check again": one read now, and reading on while the run is open. */
	check(operationId: string): void;
	/** "Stop this run". */
	stop(operationId: string): void;
	/** The sheet that showed it closed: an ended run's output leaves this page's memory. */
	forgetOutput(operationId: string): void;
}

interface Watch {
	timer?: ReturnType<typeof setTimeout>;
	release?: () => void;
	failures: number;
	since: number;
}

type EndedState = Exclude<EventRun["run"], "queued" | "running">;

const ITEM_ENDS: Record<
	EndedState,
	{ outcome: "done" | "failed"; detail: "done" | "failed" | "cancelled" }
> = {
	succeeded: { outcome: "done", detail: "done" },
	failed: { outcome: "failed", detail: "failed" },
	timed_out: { outcome: "failed", detail: "failed" },
	cancelled: { outcome: "failed", detail: "cancelled" },
};

/** Errors thrown before the command left this computer: nothing ran. */
const notSent = (error: unknown) =>
	error instanceof ManagementRequestNotSentError ||
	error instanceof LiveCallError ||
	error instanceof ConnectError ||
	error instanceof RangeError;

function trayTarget(source: RunStart | RunLookup): ActivityTarget {
	const target: ActivityTarget = {
		deviceId: source.deviceId,
		serviceId: source.serviceId,
		eventId: source.eventId,
	};
	if ("deviceName" in source && source.deviceName)
		target.deviceName = source.deviceName;
	if ("projectId" in source && source.projectId)
		target.projectId = source.projectId;
	return target;
}

const openPatch = (run: EventRun): Partial<ActivityItem> =>
	run.run === "queued"
		? {
				state: "waiting",
				detail: { code: "waiting_for_device" },
				progress: "indeterminate",
			}
		: { state: "active", detail: undefined, progress: "indeterminate" };

const withoutOutput = (run: EventRun): EventRun =>
	Object.fromEntries(
		Object.entries(run).filter(([key]) => key !== "output"),
	) as unknown as EventRun;

function trayItem(
	operationId: string,
	source: RunStart,
	patch: Partial<ActivityStart>,
): ActivityStart {
	return {
		kind: "event_run",
		target: trayTarget(source),
		state: "active",
		label: { code: "event_run" },
		startedBy: "you",
		resume: {
			type: "operation",
			operationId,
			command: "run_event",
			issuedAt: Math.floor(Date.now() / 1000),
		},
		// This page reads an open run itself; the handle lets the tray look it up after a reload.
		actions: [],
		...patch,
	};
}

function createRunStore(workspace: DeviceWorkspace): RunStore {
	const entries = new Map<string, RunEntry>();
	const items = new Map<string, string>();
	const watches = new Map<string, Watch>();
	const listeners = new Set<() => void>();
	const { activity, live, keys } = workspace;

	const emit = () => {
		for (const listener of [...listeners]) listener();
	};
	const put = (operationId: string, patch: Partial<RunEntry>) => {
		const entry = entries.get(operationId);
		if (!entry) return;
		entries.set(operationId, { ...entry, ...patch });
		emit();
	};
	const callOf = (
		deviceId: string,
		lane: CallLane,
		options: { idempotent?: boolean } = {},
	) => {
		const call = live.call(deviceId, { lane, ...options });
		return (command: Record<string, unknown>, operationId?: string) => {
			workspace.touch(deviceId);
			return call(command, operationId);
		};
	};

	/** The tray item of a run: ours, or the "no reply" item the live session filed for its operation id. */
	function itemOf(operationId: string): string | undefined {
		const known = items.get(operationId);
		if (known) return known;
		const filed = activity
			.list()
			.find(
				(item) =>
					item.resume?.type === "operation" &&
					item.resume.operationId === operationId,
			);
		if (filed) items.set(operationId, filed.id);
		return filed?.id;
	}

	function track(operationId: string, run: EventRun, source?: RunStart) {
		const itemId = itemOf(operationId);
		if (!itemId) {
			if (source && runOpen(run))
				items.set(
					operationId,
					activity.start(trayItem(operationId, source, openPatch(run))),
				);
			return;
		}
		if (runOpen(run)) {
			activity.update(itemId, openPatch(run));
			return;
		}
		const end = ITEM_ENDS[run.run as EndedState];
		activity.finish(itemId, end.outcome, { code: end.detail });
	}

	function unwatch(operationId: string) {
		const watch = watches.get(operationId);
		if (!watch) return;
		clearTimeout(watch.timer);
		watch.release?.();
		watches.delete(operationId);
	}

	function schedule(operationId: string) {
		const watch = watches.get(operationId);
		if (!watch) return;
		const fast = Date.now() - watch.since < runPace.fastForMs;
		watch.timer = setTimeout(
			() => void read(operationId),
			fast ? runPace.fastMs : runPace.slowMs,
		);
	}

	function watch(operationId: string) {
		const entry = entries.get(operationId);
		if (!entry || watches.has(operationId)) return;
		watches.set(operationId, {
			failures: 0,
			since: entry.since,
			release: live.acquire(entry.deviceId, "operation"),
		});
		schedule(operationId);
	}

	function apply(operationId: string, run: EventRun, source?: RunStart) {
		track(operationId, run, source);
		const clear = { readFailure: undefined, paused: false };
		if (runOpen(run)) {
			put(operationId, { phase: { kind: "open", run }, ...clear });
			watch(operationId);
			if (watches.get(operationId)?.timer === undefined) schedule(operationId);
			return;
		}
		unwatch(operationId);
		put(operationId, {
			phase: { kind: "ended", run },
			...clear,
			stopping: false,
		});
	}

	function forgotten(operationId: string) {
		unwatch(operationId);
		put(operationId, { phase: { kind: "unknown" }, readFailure: undefined });
		const itemId = itemOf(operationId);
		if (itemId) activity.finish(itemId, "failed", { code: "failed" });
	}

	function readFailed(operationId: string, error: unknown) {
		const failure = classifyDeviceError(error);
		const watching = watches.get(operationId);
		if (watching && ++watching.failures < runPace.maxReadFailures) {
			put(operationId, { readFailure: failure });
			schedule(operationId);
			return;
		}
		unwatch(operationId);
		put(operationId, {
			readFailure: failure,
			paused: entries.get(operationId)?.phase.kind === "open",
		});
	}

	async function read(operationId: string) {
		const entry = entries.get(operationId);
		if (!entry) return;
		const watching = watches.get(operationId);
		// One read at a time: a read asked for in between (Stop) replaces the scheduled one.
		if (watching) {
			clearTimeout(watching.timer);
			watching.timer = undefined;
		}
		if (keys.snapshot(entry.deviceId).state !== "unlocked") {
			unwatch(operationId);
			put(operationId, { paused: entry.phase.kind === "open" });
			return;
		}
		try {
			const run = await readRun(
				callOf(entry.deviceId, "operation"),
				operationId,
			);
			if (watching) watching.failures = 0;
			if (run) apply(operationId, run);
			else forgotten(operationId);
		} catch (error) {
			readFailed(operationId, error);
		}
	}

	function unanswered(operationId: string, start: RunStart, error: unknown) {
		if (notSent(error)) {
			put(operationId, {
				phase: { kind: "not_sent", failure: classifyDeviceError(error) },
			});
			return;
		}
		// The live session files its own "no reply" item; any other doubt gets one here.
		if (!(error instanceof ManagementUnconfirmedError))
			activity.finish(
				activity.start(
					trayItem(operationId, start, {
						state: "unknown",
						detail: { code: "no_reply" },
						actions: ["check_again", "dismiss"],
					}),
				),
				"unknown",
				{ code: "no_reply" },
			);
		put(operationId, { phase: { kind: "no_reply" } });
	}

	async function send(operationId: string, start: RunStart) {
		const call = live.call(start.deviceId, {
			lane: "user",
			trackUnconfirmed: { kind: "event_run", target: trayTarget(start) },
		});
		workspace.touch(start.deviceId);
		try {
			const answer = await runEvent(
				call,
				{
					placementId: start.serviceId,
					eventId: start.eventId,
					expectedRevision: start.expectedRevision,
					...(start.payload ? { payload: start.payload } : {}),
				},
				operationId,
			);
			if (answer.kind === "accepted") apply(operationId, answer.run, start);
			else
				put(operationId, {
					phase: {
						kind: "rejected",
						code: answer.code,
						rejection: answer.rejection,
					},
				});
		} catch (error) {
			unanswered(operationId, start, error);
		}
	}

	function stop(operationId: string) {
		const entry = entries.get(operationId);
		if (entry?.phase.kind !== "open" || entry.stopping) return;
		put(operationId, { stopping: true, stopFailure: undefined });
		cancelRun(
			callOf(entry.deviceId, "user", { idempotent: true }),
			operationId,
			crypto.randomUUID(),
		).then(
			(cancelled) => {
				if (!cancelled) void read(operationId);
			},
			(error) =>
				put(operationId, {
					stopping: false,
					stopFailure: classifyDeviceError(error),
				}),
		);
	}

	return {
		get: (operationId) => entries.get(operationId),
		subscribe(listener) {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
		start(start) {
			const operationId = crypto.randomUUID();
			entries.set(operationId, {
				operationId,
				deviceId: start.deviceId,
				serviceId: start.serviceId,
				eventId: start.eventId,
				phase: { kind: "sending" },
				since: Date.now(),
			});
			emit();
			void send(operationId, start);
			return operationId;
		},
		view(lookup) {
			const known = entries.get(lookup.operationId);
			const phase = known?.phase;
			if (phase?.kind === "sending" || watches.has(lookup.operationId)) return;
			if (phase?.kind === "ended" && phase.run.output !== undefined) return;
			entries.set(lookup.operationId, {
				...lookup,
				phase: phase ?? { kind: "reading" },
				since: known?.since ?? Date.now(),
			});
			emit();
			void read(lookup.operationId);
		},
		check(operationId) {
			const entry = entries.get(operationId);
			if (!entry || watches.has(operationId)) return;
			put(operationId, {
				paused: false,
				readFailure: undefined,
				...(entry.phase.kind === "no_reply"
					? { phase: { kind: "reading" } }
					: {}),
			});
			void read(operationId);
		},
		stop,
		forgetOutput(operationId) {
			const phase = entries.get(operationId)?.phase;
			if (phase?.kind !== "ended" || phase.run.output === undefined) return;
			put(operationId, {
				phase: { kind: "ended", run: withoutOutput(phase.run) },
			});
		},
	};
}

const stores = new WeakMap<DeviceWorkspace, RunStore>();

/** The runs of one workspace: they outlive the sheet that started them, not the account. */
export function runsOf(workspace: DeviceWorkspace): RunStore {
	let store = stores.get(workspace);
	if (!store) {
		store = createRunStore(workspace);
		stores.set(workspace, store);
	}
	return store;
}
