import { describe, expect, test } from "bun:test";
import {
	type ExecuteEventFn,
	ExecutionEngineProvider,
} from "../../../../lib/execution-engine";
import type { IIntercomEvent } from "../../../../lib/schema/events/intercom-event";
import type { IBackendState } from "../../../../state/backend-state";
import type { IPricingResponse } from "../../../../state/backend-state/user-state";
import {
	type FieldValue,
	type FileSlot,
	type FormModel,
	type FormSessionActions,
	type FormSessionState,
	type InitialSessionState,
	type NavigateIntent,
	REQUEST_FILES_STORE_REF,
	type ReduceSession,
	type RunEntry,
	type RunOutcome,
	type RunStatus,
	type SessionCommand,
	type SessionEffect,
	type SessionInput,
	type SessionMessage,
	type SessionStep,
	type StoredRunRecord,
	type StoredRunStatus,
} from "../contracts";
import { EMPTY_RUN_SUMMARY } from "../run/summary";
import { createFormMemoryStore } from "../store/form-memory";
import {
	FIXTURE_APP_HOST,
	FIXTURE_HOSTED_HOST,
	fixture,
	fixtureForm,
} from "../testing/fixtures";
import {
	type FormSessionController,
	type RuntimeBackend,
	type RuntimeTimers,
	createSessionController,
	localDay,
	sessionActions,
	tierLimitOf,
} from "./controller";
import { OUTPUT_FRAME_MS } from "./dispatch-run";
import { reduceSession } from "./reduce";
import {
	attachSessionController,
	detachSessionController,
	getSessionController,
} from "./registry";
import { initialSessionState } from "./state";

// ─── Manual time ────────────────────────────────────────────────────────────

const BASE = Date.UTC(2026, 9, 5, 12, 0, 0);

class ManualTimers implements RuntimeTimers {
	now = 0;
	private seq = 0;
	private readonly pending = new Map<
		number,
		{ readonly at: number; readonly callback: () => void }
	>();

	setTimeout = (callback: () => void, ms: number): unknown => {
		this.seq += 1;
		this.pending.set(this.seq, { at: this.now + ms, callback });
		return this.seq;
	};

	clearTimeout = (handle: unknown): void => {
		this.pending.delete(handle as number);
	};

	advance(ms: number): void {
		const end = this.now + ms;
		for (let next = this.due(end); next; next = this.due(end)) {
			this.pending.delete(next[0]);
			this.now = next[1].at;
			next[1].callback();
		}
		this.now = end;
	}

	private due(end: number) {
		return [...this.pending.entries()]
			.filter(([, timer]) => timer.at <= end)
			.sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
	}
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function deferred<T>() {
	let resolve: (value: T) => void = () => {};
	const promise = new Promise<T>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

let eventSeq = 0;
const event = (event_type: string, payload: unknown): IIntercomEvent => {
	eventSeq += 1;
	return {
		event_id: `event-${eventSeq}`,
		event_type,
		payload,
		timestamp: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	};
};

// ─── A scripted reducer: just enough of S-STATE to drive the runtime ────────

type Step = (state: FormSessionState) => SessionStep;
const keep = (state: FormSessionState, effects: SessionEffect[] = []) => ({
	state,
	effects,
});

const BUSY: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"starting",
	"asking",
	"running",
	"streaming",
]);
const STATUS_OF: Readonly<Record<RunOutcome["kind"], RunStatus>> = {
	succeeded: "done",
	failed: "failed",
	stopped: "stopped",
	notStarted: "notStarted",
	noPlace: "queued",
	unknown: "unknown",
};
const STORED: Partial<Record<RunStatus, StoredRunStatus>> = {
	done: "done",
	empty: "empty",
	failed: "failed",
	stopped: "stopped",
	notStarted: "notStarted",
	queued: "queued",
	sending: "sending",
};

function recordOf(run: RunEntry, form: FormModel): StoredRunRecord {
	return {
		version: 2,
		id: run.id,
		scope: form.host.memoryScope ?? "session",
		appId: form.appId,
		eventId: form.eventId,
		n: run.n,
		createdAt: run.createdAt,
		startedAt: run.startedAt,
		endedAt: run.endedAt,
		status: STORED[run.status] ?? "running",
		outcome: run.outcome,
		summary: run.summary,
		failedAt: null,
		presetName: null,
		inputs: {},
		perRun: [],
		replaced: [],
	};
}

function mapRun(
	state: FormSessionState,
	id: string,
	update: (run: RunEntry) => RunEntry,
) {
	return {
		...state,
		runs: state.runs.map((run) => (run.id === id ? update(run) : run)),
	};
}

const withValue = (
	state: FormSessionState,
	name: string,
	value: FieldValue,
) => ({
	...state,
	rail: { ...state.rail, values: { ...state.rail.values, [name]: value } },
});

function mapSlot(
	state: FormSessionState,
	id: string,
	update: (slot: FileSlot) => FileSlot,
) {
	const values = Object.fromEntries(
		Object.entries(state.rail.values).map(([name, value]) => {
			const slot = value as FileSlot | null;
			return [name, slot && slot.id === id ? update(slot) : value];
		}),
	);
	return { ...state, rail: { ...state.rail, values } };
}

function addRun(state: FormSessionState, now: number): SessionStep {
	const n = state.runs.length + 1;
	const run: RunEntry = {
		id: `run-${n}`,
		n,
		origin: "session",
		status: "starting",
		createdAt: now,
		startedAt: null,
		endedAt: null,
		copy: {
			values: state.rail.values,
			presetName: null,
			leftAsIs: false,
			perRun: [],
			replaced: [],
		},
		target: state.queue.target,
		streamId: `run-${n}#1`,
		backendRunId: null,
		output: null,
		outcome: null,
		summary: EMPTY_RUN_SUMMARY,
		stopRequested: false,
		pendingSlotIds: [],
		waitingForPlace: false,
		failedAt: null,
		unseenFailure: false,
	};
	return keep({ ...state, runs: [run, ...state.runs] }, [
		{ type: "dispatchRun", runId: run.id },
	]);
}

function pick(state: FormSessionState, command: SessionCommand): SessionStep {
	if (command.type !== "pickFiles") return keep(state);
	const [file] = command.files;
	const field = state.form.fields.find((item) => item.name === command.name);
	const slot: FileSlot = {
		id: file.slotId,
		name: file.name,
		size: file.size,
		type: file.type || null,
		state: "sending",
		progress: 0,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
	return keep(withValue(state, command.name, slot), [
		{
			type: "upload",
			slotId: file.slotId,
			name: command.name,
			mode: field?.fileMode ?? "url",
		},
	]);
}

function commandStep(
	state: FormSessionState,
	command: SessionCommand,
	now: number,
): SessionStep {
	const steps: Partial<Record<SessionCommand["type"], Step>> = {
		run: (current) => addRun(current, now),
		stop: (current) => {
			if (command.type !== "stop") return keep(current);
			return keep(
				mapRun(current, command.runId, (run) => ({
					...run,
					stopRequested: true,
				})),
				[{ type: "stopRun", runId: command.runId }],
			);
		},
		pickFiles: (current) => pick(current, command),
		removeFile: (current) => {
			if (command.type !== "removeFile") return keep(current);
			return keep(withValue(current, command.name, null), [
				{ type: "abortUpload", slotId: command.slotId },
				{ type: "releaseFiles", slotIds: [command.slotId] },
			]);
		},
		respondInteraction: (current) => {
			if (command.type !== "respondInteraction") return keep(current);
			const { runId, interactionId, value } = command;
			return keep(current, [
				{ type: "respondInteraction", runId, interactionId, value },
			]);
		},
		dismissMessage: (current) =>
			keep(current, [{ type: "expireMessage", seq: 7, afterMs: 4000 }]),
		resumeQueue: (current) =>
			keep(current, [{ type: "scheduleRetry", afterMs: 15_000 }]),
	};
	return (steps[command.type] ?? keep)(state);
}

function settledStep(
	state: FormSessionState,
	input: SessionInput,
	now: number,
) {
	if (input.type !== "runSettled") return keep(state);
	const next = mapRun(state, input.runId, (run) => ({
		...run,
		status: STATUS_OF[input.outcome.kind],
		outcome: input.outcome,
		endedAt: now,
	}));
	const run = next.runs.find((item) => item.id === input.runId);
	return keep(
		next,
		run ? [{ type: "persistRun", record: recordOf(run, next.form) }] : [],
	);
}

/** Leaving, as S-STATE does it: uploads in flight abort, every File goes. */
function leftStep(state: FormSessionState): SessionStep {
	const slots = Object.values(state.rail.values).filter(
		(value): value is FileSlot =>
			typeof value === "object" && value !== null && "state" in value,
	);
	const sending = slots.filter((slot) => slot.state === "sending");
	return keep(state, [
		...sending.map(
			(slot): SessionEffect => ({ type: "abortUpload", slotId: slot.id }),
		),
		{ type: "releaseFiles", slotIds: slots.map((slot) => slot.id) },
	]);
}

type InputStep = (
	state: FormSessionState,
	input: SessionInput,
	now: number,
) => SessionStep;

const INPUT_STEPS: Partial<Record<SessionInput["type"], InputStep>> = {
	runAccepted: (state, input) =>
		input.type !== "runAccepted"
			? keep(state)
			: keep(
					mapRun(state, input.runId, (run) => ({
						...run,
						status: BUSY.has(run.status) ? "running" : run.status,
						backendRunId: input.backendRunId,
					})),
				),
	runOutput: (state, input) =>
		input.type !== "runOutput"
			? keep(state)
			: keep(
					mapRun(state, input.runId, (run) => ({
						...run,
						output: input.output,
					})),
				),
	runSettled: settledStep,
	uploadSent: (state, input) =>
		input.type !== "uploadSent"
			? keep(state)
			: keep(
					mapSlot(state, input.slotId, (slot) => ({
						...slot,
						state: "sent",
						ref: input.ref,
						progress: null,
					})),
				),
	uploadFailed: (state, input) =>
		input.type !== "uploadFailed"
			? keep(state)
			: keep(
					mapSlot(state, input.slotId, (slot) => ({
						...slot,
						state: "failed",
						error: input.error,
					})),
				),
	formChanged: (state, input) =>
		input.type !== "formChanged"
			? keep(state)
			: keep({ ...state, form: input.form }),
	detached: leftStep,
};

const unchanged: InputStep = (state) => keep(state);

const inputStep: InputStep = (state, input, now) =>
	(INPUT_STEPS[input.type] ?? unchanged)(state, input, now);

const scripted: ReduceSession = (state, message, clock) =>
	message.kind === "command"
		? commandStep(state, message.command, clock.now)
		: inputStep(state, message.input, clock.now);

// ─── The harness ────────────────────────────────────────────────────────────

interface BackendParts {
	readonly alwaysRemote?: boolean;
	readonly canExecuteLocally?: boolean;
	readonly pricing?: () => Promise<IPricingResponse>;
	readonly upload?: RuntimeBackend["helperState"]["filesToTemporaryFiles"];
}

function fakeBackend(parts: BackendParts = {}) {
	const cancelled: string[] = [];
	const prices: number[] = [];
	const preruns: string[] = [];
	const backend: RuntimeBackend = {
		eventState: {
			alwaysRemote: parts.alwaysRemote ?? false,
			cancelExecution: async (runId) => {
				cancelled.push(runId);
			},
			prerunEvent: async (appId, eventId) => {
				preruns.push(`${appId}/${eventId}`);
				return {
					board_id: "board",
					runtime_variables: [],
					oauth_requirements: [],
					requires_local_execution: false,
					execution_mode: "Hybrid" as never,
					can_execute_locally: parts.canExecuteLocally ?? true,
				};
			},
		},
		helperState: {
			fileToUrl: async () => "data:,",
			filesToTemporaryFiles: parts.upload,
		},
		userState: {
			getPricing: () => {
				prices.push(1);
				return parts.pricing
					? parts.pricing()
					: Promise.reject(new Error("offline"));
			},
		},
	};
	return { backend, cancelled, prices, preruns };
}

let formSeq = 0;
/** A fresh event id per test: the prerun cache is shared by the module. */
function freshForm(key: "none" | "small" = "none", host = FIXTURE_APP_HOST) {
	formSeq += 1;
	const form = fixtureForm(key, host);
	return { ...form, eventId: `${form.eventId}-${formSeq}` };
}

interface HarnessOptions {
	readonly form?: FormModel;
	readonly execute?: ExecuteEventFn;
	readonly backend?: BackendParts;
	readonly reduce?: ReduceSession;
	readonly initial?: InitialSessionState;
	readonly respond?: (interaction: unknown, value: unknown) => Promise<void>;
}

const fixtureStart: InitialSessionState = (model) => ({
	...fixture(model.fields.length > 0 ? "small" : "none"),
	form: model,
});

function harness(options: HarnessOptions = {}) {
	const timers = new ManualTimers();
	const engine = new ExecutionEngineProvider();
	engine.setBackend({} as unknown as IBackendState);
	engine.setExecuteEventFn(options.execute ?? (async () => undefined));
	const memory = createFormMemoryStore("session", new Map());
	const messages: SessionMessage[] = [];
	const navigations: NavigateIntent[] = [];
	const backend = fakeBackend(options.backend);
	const form = options.form ?? freshForm();
	const reduce = options.reduce ?? scripted;
	const controller = createSessionController({
		engine,
		backend: backend.backend,
		form,
		reduce: (state, message, clock) => {
			messages.push(message);
			return reduce(state, message, clock);
		},
		initial: options.initial ?? fixtureStart,
		memory,
		navigate: (intent) => navigations.push(intent),
		now: () => BASE + timers.now,
		timers,
		respond: options.respond,
	});
	const inputs = (type: SessionInput["type"]) =>
		messages.filter(
			(message) => message.kind === "input" && message.input.type === type,
		);
	const key = {
		scope: "profile:local",
		appId: form.appId,
		eventId: form.eventId,
	};
	return {
		controller,
		engine,
		timers,
		memory,
		messages,
		navigations,
		backend,
		form,
		inputs,
		key,
	};
}

const runOf = (controller: FormSessionController, id = "run-1") =>
	controller.getState().runs.find((run) => run.id === id);

// ─── Running ────────────────────────────────────────────────────────────────

describe("a run through the controller", () => {
	test("a press is sent, accepted, fed live and settled; its record is saved", async () => {
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-1");
				cb?.([event("run_initiated", {}), event("generic_result", 42)]);
				cb?.([event("completed", { status: "completed" })]);
				return undefined;
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();

		const run = runOf(h.controller);
		expect(run?.status).toBe("done");
		expect(run?.backendRunId).toBe("backend-1");
		expect(run?.output?.result).toEqual({ value: 42 });
		expect(h.inputs("runOutput")).toHaveLength(1);
		const saved = await h.memory.load(h.key);
		expect(saved.runs.map((record) => [record.id, record.status])).toEqual([
			["run-1", "done"],
		]);
		expect(h.engine.hasStream("run-1#1")).toBe(false);
	});

	test("an error event, then completed failed, settles the run as failed", async () => {
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-2");
				cb?.([
					event("run_initiated", {}),
					event("error", { message: "No PO" }),
				]);
				cb?.([event("completed", { status: "failed" })]);
				return undefined;
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		expect(runOf(h.controller)?.status).toBe("failed");
		expect(runOf(h.controller)?.outcome).toMatchObject({
			kind: "failed",
			failure: "flow",
			message: "No PO",
		});
	});

	test("Stop on a run with its id cancels it on the host; the run ends stopped", async () => {
		const gate = deferred<void>();
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-3");
				cb?.([event("run_initiated", {})]);
				await gate.promise;
				return undefined;
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		h.controller.actions.stop("run-1");
		expect(h.backend.cancelled).toEqual(["backend-3"]);
		gate.resolve();
		await flush();
		expect(runOf(h.controller)?.status).toBe("stopped");
	});

	test("navigation is taken once: not again after detach and re-attach", async () => {
		const gate = deferred<void>();
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-4");
				cb?.([event("a2ui", { type: "navigateTo", route: "/support" })]);
				await gate.promise;
				return undefined;
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		expect(h.navigations).toEqual([{ route: "/support", replace: false }]);

		h.controller.detach();
		h.timers.advance(500);
		h.controller.attach();
		gate.resolve();
		await flush();
		expect(h.navigations).toHaveLength(1);
	});

	test("navigation during a quick remount waits for the mount; once left it is dropped", async () => {
		let send: (route: string) => void = () => {};
		const gate = deferred<void>();
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-5");
				send = (route) =>
					cb?.([event("a2ui", { type: "navigateTo", route, replace: true })]);
				await gate.promise;
				return undefined;
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		h.controller.detach();
		send("/during-remount");
		expect(h.navigations).toEqual([]);
		h.controller.attach();
		expect(h.navigations).toEqual([
			{ route: "/during-remount", replace: true },
		]);

		h.controller.detach();
		h.timers.advance(2000);
		send("/after-leaving");
		h.controller.attach();
		expect(h.navigations).toHaveLength(1);
		gate.resolve();
		await flush();
	});

	test("an answer to an in-run question is sent and the question reads responded", async () => {
		const gate = deferred<void>();
		const answered: unknown[] = [];
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-6");
				cb?.([
					event("run_initiated", {}),
					event("interaction_request", {
						id: "q-1",
						name: "Book line 4?",
						status: "pending",
						expires_at: 4_102_444_800,
					}),
				]);
				await gate.promise;
				return undefined;
			},
			respond: async (_interaction, value) => {
				answered.push(value);
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		h.controller.actions.respondInteraction("run-1", "q-1", "yes");
		await flush();
		expect(answered).toEqual(["yes"]);
		expect(runOf(h.controller)?.output?.interactions).toMatchObject([
			{ id: "q-1", status: "responded", response_value: "yes" },
		]);
		gate.resolve();
		await flush();
	});

	test("an answer that cannot be sent is reported, so the dock can say so", async () => {
		const gate = deferred<void>();
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-6b");
				cb?.([
					event("run_initiated", {}),
					event("interaction_request", {
						id: "q-1",
						name: "Book line 4?",
						status: "pending",
						expires_at: 4_102_444_800,
					}),
				]);
				await gate.promise;
				return undefined;
			},
			respond: async () => {
				throw new Error("channel closed");
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		h.controller.actions.respondInteraction("run-1", "q-1", "yes");
		await flush();
		expect(h.inputs("interactionFailed")).toEqual([
			{
				kind: "input",
				input: {
					type: "interactionFailed",
					runId: "run-1",
					interactionId: "q-1",
					error: "Error: channel closed",
				},
			},
		]);
		gate.resolve();
		await flush();
		h.controller.actions.respondInteraction("run-1", "q-2", "no");
		expect(h.inputs("interactionFailed")).toHaveLength(2);
	});

	test("a streamed answer reaches the session once a frame, and all of it before the run settles", async () => {
		const gate = deferred<void>();
		let chunk: (text: string) => void = () => {};
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-6c");
				chunk = (text) => cb?.([event("chunk", { text })]);
				cb?.([event("run_initiated", {})]);
				await gate.promise;
				cb?.([event("completed", { status: "completed" })]);
				return undefined;
			},
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		let renders = 0;
		h.controller.subscribe(() => {
			renders += 1;
		});
		h.timers.advance(OUTPUT_FRAME_MS);
		expect(h.inputs("runOutput")).toHaveLength(1);
		for (const text of ["Ven", "dor", ": ", "Nord", "wind"]) chunk(text);
		expect(h.inputs("runOutput")).toHaveLength(1);
		h.timers.advance(OUTPUT_FRAME_MS);
		expect(h.inputs("runOutput")).toHaveLength(2);
		expect(runOf(h.controller)?.output?.eventCount).toBe(6);
		chunk("!");
		gate.resolve();
		await flush();
		const types = h.messages
			.filter((message) => message.kind === "input")
			.map((message) => (message.kind === "input" ? message.input.type : ""));
		expect(types.slice(-2)).toEqual(["runOutput", "runSettled"]);
		expect(runOf(h.controller)?.output?.eventCount).toBe(8);
		expect(renders).toBe(4);
	});
});

// ─── Attach, detach, leave ──────────────────────────────────────────────────

describe("attach, detach and leave", () => {
	test("attached, the live stream stays out of the background card; left, it shows", async () => {
		const gate = deferred<void>();
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId) => {
				onEventId?.("backend-7");
				await gate.promise;
				return undefined;
			},
		});
		const background = () =>
			h.engine.getBackgroundStreams().map((stream) => stream.streamId);
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		expect(background()).toEqual([]);

		h.controller.detach();
		h.timers.advance(1999);
		expect(background()).toEqual([]);
		expect(h.inputs("detached")).toHaveLength(0);
		h.timers.advance(1);
		expect(h.inputs("detached")).toHaveLength(1);
		expect(background()).toEqual(["run-1#1"]);

		h.controller.attach();
		expect(background()).toEqual([]);
		gate.resolve();
		await flush();
	});

	test("the leave grace survives a quick remount", () => {
		const h = harness();
		h.controller.attach();
		h.controller.detach();
		h.timers.advance(1500);
		h.controller.attach();
		h.timers.advance(10_000);
		expect(h.inputs("detached")).toHaveLength(0);
		expect(h.inputs("attached")).toHaveLength(2);
		expect(h.controller.isAttached()).toBe(true);
	});

	test("a run that settles after the form was left records its end and its stream goes", async () => {
		const gate = deferred<void>();
		const h = harness({
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-8");
				cb?.([event("run_initiated", {})]);
				await gate.promise;
				cb?.([event("completed", { status: "completed" })]);
				return undefined;
			},
		});
		let disposed = 0;
		h.controller.onDispose(() => {
			disposed += 1;
		});
		h.controller.attach();
		h.controller.actions.run();
		await flush();
		h.controller.detach();
		h.timers.advance(2000);
		expect(h.engine.hasStream("run-1#1")).toBe(true);
		expect(disposed).toBe(0);

		gate.resolve();
		await flush();
		expect(h.engine.hasStream("run-1#1")).toBe(false);
		expect(h.engine.getBackgroundStreams()).toEqual([]);
		const saved = await h.memory.load(h.key);
		expect(saved.runs.map((record) => record.status)).toEqual(["done"]);
		expect(disposed).toBe(1);
		expect(h.controller.isDisposed()).toBe(true);
	});

	test("leaving aborts uploads still in flight; their late results are dropped", async () => {
		let signal: AbortSignal | undefined;
		const h = harness({
			form: freshForm("small"),
			backend: {
				upload: (_files, options) =>
					new Promise((_resolve, reject) => {
						signal = options?.signal;
						signal?.addEventListener("abort", () =>
							reject(new DOMException("Aborted", "AbortError")),
						);
					}),
			},
		});
		h.controller.attach();
		h.controller.actions.pickFiles(
			"receipt",
			[new File(["%PDF"], "receipt.pdf", { type: "application/pdf" })],
			"replace",
		);
		await flush();
		expect(signal?.aborted).toBe(false);
		h.controller.detach();
		h.timers.advance(2000);
		await flush();
		expect(signal?.aborted).toBe(true);
		expect(h.inputs("uploadFailed")).toEqual([]);
		expect(h.controller.isDisposed()).toBe(true);
	});

	test("a form left with nothing running ends at once; later messages are ignored", () => {
		const h = harness();
		h.controller.attach();
		h.controller.detach();
		h.timers.advance(2000);
		expect(h.controller.isDisposed()).toBe(true);
		const before = h.messages.length;
		h.controller.actions.run();
		expect(h.messages).toHaveLength(before);
	});

	test("memory loads and the target resolves once, on the first mount", async () => {
		const h = harness();
		expect(h.inputs("memoryLoaded")).toHaveLength(0);
		h.controller.attach();
		h.controller.detach();
		h.controller.attach();
		await flush();
		expect(h.inputs("memoryLoaded")).toHaveLength(1);
		expect(h.backend.preruns).toHaveLength(1);
		expect(
			h.inputs("targetResolved").map((m) => m.kind === "input" && m.input),
		).toEqual([{ type: "targetResolved", target: "local", tierLimit: null }]);
		expect(h.backend.prices).toEqual([]);
	});

	test("in the cloud the tier's limit follows the target", async () => {
		const pricing: IPricingResponse = {
			current_tier: "PRO",
			tiers: {
				PRO: {
					name: "PRO",
					max_non_visible_projects: 0,
					max_remote_executions: 0,
					max_concurrent_executions: 20,
					execution_tier: "pro",
					max_total_size: 0,
					max_llm_cost: 0,
					llm_tiers: [],
				},
			},
		};
		const h = harness({
			backend: { alwaysRemote: true, pricing: async () => pricing },
		});
		h.controller.attach();
		await flush();
		expect(
			h.inputs("targetResolved").map((m) => m.kind === "input" && m.input),
		).toEqual([
			{ type: "targetResolved", target: "remote", tierLimit: null },
			{ type: "targetResolved", target: "remote", tierLimit: 20 },
		]);
		expect(h.backend.preruns).toEqual([]);
	});

	test("a hosted page reports its fixed target and asks nothing", async () => {
		const h = harness({ form: freshForm("none", FIXTURE_HOSTED_HOST) });
		h.controller.attach();
		await flush();
		expect(
			h.inputs("targetResolved").map((m) => m.kind === "input" && m.input),
		).toEqual([{ type: "targetResolved", target: "remote", tierLimit: null }]);
		expect(h.backend.preruns).toEqual([]);
		expect(h.backend.prices).toEqual([]);
	});
});

// ─── Files ──────────────────────────────────────────────────────────────────

describe("files", () => {
	const FLOW_PATH = {
		path: "tmp/global/apps/a/events/e/requests/r/0001-receipt.pdf",
		store_ref: REQUEST_FILES_STORE_REF,
		cache_store_ref: null,
	};
	const receipt = () =>
		new File(["%PDF"], "receipt.pdf", { type: "application/pdf" });
	const slotOf = (controller: FormSessionController) =>
		controller.getState().rail.values.receipt as FileSlot | null;

	test("a picked file uploads for this event and target and becomes sent", async () => {
		const seen: Record<string, unknown>[] = [];
		const h = harness({
			form: freshForm("small"),
			backend: {
				upload: async (files, options) => {
					seen.push({ ...options, name: files[0].name });
					return [
						{ file: files[0], uploaded: { url: "", flowPath: FLOW_PATH } },
					];
				},
			},
		});
		h.controller.attach();
		h.controller.actions.pickFiles("receipt", [receipt()], "replace");
		await flush();
		expect(seen).toMatchObject([
			{
				name: "receipt.pdf",
				appId: h.form.appId,
				eventId: h.form.eventId,
				executionTarget: "local",
			},
		]);
		expect(slotOf(h.controller)).toMatchObject({
			state: "sent",
			ref: { kind: "flowpath", flowPath: FLOW_PATH, url: null },
		});
	});

	test("removing a file while it is sent aborts its upload", async () => {
		let signal: AbortSignal | undefined;
		const h = harness({
			form: freshForm("small"),
			backend: {
				upload: (_files, options) =>
					new Promise((_resolve, reject) => {
						signal = options?.signal;
						signal?.addEventListener("abort", () =>
							reject(new DOMException("Aborted", "AbortError")),
						);
					}),
			},
		});
		h.controller.attach();
		h.controller.actions.pickFiles("receipt", [receipt()], "replace");
		await flush();
		const id = slotOf(h.controller)?.id ?? "";
		h.controller.actions.removeFile("receipt", id);
		await flush();
		expect(signal?.aborted).toBe(true);
		expect(h.inputs("uploadFailed")).toEqual([]);
	});

	test("a failed upload reports its words; a slot without its File fails at once", async () => {
		const h = harness({
			form: freshForm("small"),
			backend: {
				upload: async (files) => [{ file: files[0], error: "Network down" }],
			},
		});
		h.controller.attach();
		h.controller.actions.pickFiles("receipt", [receipt()], "replace");
		await flush();
		expect(slotOf(h.controller)).toMatchObject({
			state: "failed",
			error: "Network down",
		});

		const withUpload: ReduceSession = (state, message) => ({
			state,
			effects:
				message.kind === "input" && message.input.type === "attached"
					? [
							{
								type: "upload",
								slotId: "ghost",
								name: "receipt",
								mode: "flowpath",
							},
						]
					: [],
		});
		const ghost = harness({ form: freshForm("small"), reduce: withUpload });
		ghost.controller.attach();
		await flush();
		expect(
			ghost.inputs("uploadFailed").map((m) => m.kind === "input" && m.input),
		).toContainEqual({
			type: "uploadFailed",
			slotId: "ghost",
			error: "The picked file is no longer available. Pick it again.",
		});
	});
});

// ─── Timers, forms, actions ─────────────────────────────────────────────────

describe("timers, form changes and actions", () => {
	test("message expiry and the retry of a refused start come back as inputs", () => {
		const h = harness();
		h.controller.attach();
		h.controller.actions.dismissMessage();
		h.controller.actions.resumeQueue();
		h.timers.advance(4000);
		expect(
			h.inputs("messageExpired").map((m) => m.kind === "input" && m.input),
		).toEqual([{ type: "messageExpired", seq: 7 }]);
		h.timers.advance(11_000);
		expect(h.inputs("retryDue")).toHaveLength(0);
		h.timers.advance(50);
		expect(h.inputs("retryDue")).toHaveLength(1);
	});

	test("formChanged only when the content, node, host or viewer differ", () => {
		const h = harness();
		h.controller.setForm({ ...h.form });
		expect(h.inputs("formChanged")).toHaveLength(0);
		h.controller.setForm({ ...h.form, contentKey: "changed" });
		expect(h.inputs("formChanged")).toHaveLength(1);
		h.controller.setForm({
			...h.form,
			contentKey: "changed",
			host: { ...h.form.host, presentation: "tile" },
		});
		expect(h.inputs("formChanged")).toHaveLength(2);
		expect(h.controller.getState().form.host.presentation).toBe("tile");
	});

	test("a reducer that throws keeps the state and does not throw into the UI", () => {
		const errors: unknown[] = [];
		const original = console.error;
		console.error = (...args: unknown[]) => {
			errors.push(args);
		};
		try {
			const h = harness({
				reduce: () => {
					throw new Error("arrives with lane S-STATE");
				},
			});
			const before = h.controller.getState();
			expect(() => h.controller.actions.run()).not.toThrow();
			expect(h.controller.getState()).toBe(before);
			expect(errors).toHaveLength(1);
		} finally {
			console.error = original;
		}
	});

	test("listeners hear each change once; the actions object never changes", () => {
		const h = harness();
		const actions = h.controller.actions;
		let heard = 0;
		const stop = h.controller.subscribe(() => {
			heard += 1;
		});
		h.controller.actions.run();
		expect(heard).toBe(1);
		h.controller.actions.markSeen("run-1");
		expect(heard).toBe(1);
		stop();
		h.controller.actions.run();
		expect(heard).toBe(1);
		expect(h.controller.actions).toBe(actions);
	});

	test("every action sends its own command", () => {
		const sent: SessionCommand[] = [];
		const file = new File(["x"], "x.txt", { type: "text/plain" });
		const actions = sessionActions(
			(command) => sent.push(command),
			(files) =>
				files.map((item) => ({
					slotId: "slot",
					name: item.name,
					size: item.size,
					type: item.type,
				})),
		);
		const calls: readonly ((all: FormSessionActions) => void)[] = [
			(a) => a.setValue("k", "v", "replace"),
			(a) => a.setValue("k", "v"),
			(a) => a.pickFiles("receipt", [file], "append"),
			(a) => a.run(),
			(a) => a.run({ leaveAsIs: true, from: "chord" }),
			(a) => a.selectRun("run-1", "tab"),
			(a) =>
				a.savePreset({
					name: "P",
					openDefault: false,
					ticked: [],
					fromRunId: null,
					replaceId: null,
				}),
		];
		for (const call of calls) call(actions);
		expect(sent).toEqual([
			{ type: "setValue", key: "k", value: "v", how: "replace" },
			{ type: "setValue", key: "k", value: "v" },
			{
				type: "pickFiles",
				name: "receipt",
				files: [{ slotId: "slot", name: "x.txt", size: 1, type: file.type }],
				mode: "append",
			},
			{ type: "run", leaveAsIs: false, from: "button" },
			{ type: "run", leaveAsIs: true, from: "chord" },
			{ type: "selectRun", runId: "run-1", how: "tab" },
			{
				type: "savePreset",
				draft: {
					name: "P",
					openDefault: false,
					ticked: [],
					fromRunId: null,
					replaceId: null,
				},
			},
		]);
		expect(Object.keys(actions)).toHaveLength(49);
	});

	test("the clock's day is the local calendar day", () => {
		expect(localDay(new Date(2026, 0, 9, 23, 59))).toBe("2026-01-09");
	});

	test("the tier's limit is read best effort", async () => {
		const tier = (max?: number) => ({
			getPricing: async () =>
				({
					current_tier: "FREE",
					tiers: { FREE: { max_concurrent_executions: max } },
				}) as unknown as IPricingResponse,
		});
		expect(await tierLimitOf(tier(2))).toBe(2);
		expect(await tierLimitOf(tier(-1))).toBe(-1);
		expect(await tierLimitOf(tier())).toBeNull();
		expect(
			await tierLimitOf({
				getPricing: () => {
					throw new Error("signed out");
				},
			}),
		).toBeNull();
	});
});

// ─── With S-STATE's reducer ─────────────────────────────────────────────────

describe("with the session reducer", () => {
	const real = { reduce: reduceSession, initial: initialSessionState };

	test("a form without fields runs to Done, saved on this device, its stream gone", async () => {
		const h = harness({
			...real,
			execute: async (_a, _e, _p, _s, onEventId, cb) => {
				onEventId?.("backend-e2e-1");
				cb?.([event("run_initiated", {}), event("generic_result", "Triaged")]);
				cb?.([event("completed", { status: "completed" })]);
				return undefined;
			},
		});
		h.controller.attach();
		await flush();
		expect(h.controller.getState().memory.loaded).toBe(true);
		expect(h.controller.getState().queue).toMatchObject({
			target: "local",
			cap: 3,
		});

		h.controller.actions.run({ from: "hero" });
		await flush();
		const [run] = h.controller.getState().runs;
		expect(run).toMatchObject({
			status: "done",
			backendRunId: "backend-e2e-1",
			outcome: { kind: "succeeded" },
		});
		expect(run.output?.result).toEqual({ value: "Triaged" });
		const saved = await h.memory.load(h.key);
		expect(saved.runs.map((record) => [record.id, record.status])).toEqual([
			[run.id, "done"],
		]);
		expect(run.streamId).not.toBeNull();
		expect(h.engine.hasStream(run.streamId ?? "")).toBe(false);
	});

	test("a run pressed while its file uploads waits, then sends the file's FlowPath", async () => {
		const FLOW_PATH = {
			path: "tmp/global/apps/a/events/e/requests/r/0001-receipt.pdf",
			store_ref: REQUEST_FILES_STORE_REF,
			cache_store_ref: null,
		};
		const gate = deferred<void>();
		const sent: unknown[] = [];
		const h = harness({
			...real,
			form: freshForm("small"),
			backend: {
				upload: async (files) => {
					await gate.promise;
					return [
						{ file: files[0], uploaded: { url: "", flowPath: FLOW_PATH } },
					];
				},
			},
			execute: async (_a, _e, payload, _s, onEventId, cb) => {
				sent.push(payload.payload);
				onEventId?.("backend-e2e-2");
				cb?.([
					event("run_initiated", {}),
					event("completed", { status: "completed" }),
				]);
				return undefined;
			},
		});
		h.controller.attach();
		await flush();
		h.controller.actions.setValue("order", "A-1", "replace");
		h.controller.actions.pickFiles(
			"receipt",
			[new File(["%PDF"], "receipt.pdf", { type: "application/pdf" })],
			"replace",
		);
		h.controller.actions.run();
		expect(h.controller.getState().runs[0]?.status).toBe("sending");
		await flush();
		expect(sent).toEqual([]);

		gate.resolve();
		await flush();
		expect(sent).toEqual([{ order: "A-1", quantity: 1, receipt: FLOW_PATH }]);
		expect(h.controller.getState().runs[0]?.status).toBe("empty");
	});
});

// ─── The registry ───────────────────────────────────────────────────────────

describe("the registry", () => {
	const make =
		(timers = new ManualTimers()) =>
		() =>
			createSessionController({
				engine: new ExecutionEngineProvider(),
				backend: fakeBackend().backend,
				form: fixtureForm("none"),
				reduce: scripted,
				initial: (model) => ({ ...fixture("none"), form: model }),
				memory: createFormMemoryStore("session", new Map()),
				timers,
				now: () => BASE,
			});

	test("a remount gets the same controller; a second mount at once gets its own", () => {
		const engine = {};
		const factory = make();
		const first = getSessionController(engine, "app", "event", factory);
		expect(getSessionController(engine, "app", "event", factory)).toBe(first);
		expect(
			attachSessionController(engine, "app", "event", first, factory),
		).toBe(first);

		const second = attachSessionController(
			engine,
			"app",
			"event",
			first,
			factory,
		);
		expect(second).not.toBe(first);
		expect(second.isAttached()).toBe(true);
		expect(getSessionController(engine, "app", "event", factory)).toBe(first);

		detachSessionController(first);
		const third = attachSessionController(
			engine,
			"app",
			"event",
			first,
			factory,
		);
		expect(third).toBe(first);
		expect(
			attachSessionController(engine, "app", "event", second, factory),
		).toBe(second);
	});

	test("engines, events and partitions never share a controller", () => {
		const one = {};
		const two = {};
		const factory = make();
		const base = getSessionController(one, "app", "event", factory);
		expect(getSessionController(two, "app", "event", factory)).not.toBe(base);
		expect(getSessionController(one, "app", "other", factory)).not.toBe(base);
		expect(
			getSessionController(one, "app", "event", factory, "app:profile:2"),
		).not.toBe(base);
	});

	test("an ended controller leaves the registry; the next mount gets a fresh one", () => {
		const engine = {};
		const timers = new ManualTimers();
		const factory = make(timers);
		const first = attachSessionController(
			engine,
			"app",
			"event",
			getSessionController(engine, "app", "event", factory),
			factory,
		);
		detachSessionController(first);
		timers.advance(2000);
		expect(first.isDisposed()).toBe(true);
		const next = getSessionController(engine, "app", "event", factory);
		expect(next).not.toBe(first);
		expect(
			attachSessionController(engine, "app", "event", first, factory),
		).toBe(next);
	});
});
