import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
	type ApiResponseError,
	PLAN_LIMIT_EVENT,
	apiResponseError,
} from "../../../../lib/api-error";
import {
	type ExecuteEventFn,
	ExecutionEngineProvider,
} from "../../../../lib/execution-engine";
import type { IIntercomEvent } from "../../../../lib/schema/events/intercom-event";
import type { ILogMetadata } from "../../../../lib/schema/flow/log-metadata";
import type { IRunPayload } from "../../../../lib/schema/flow/run-payload";
import type { IInteractionRequest } from "../../../../lib/schema/interaction";
import type { IBackendState } from "../../../../state/backend-state";
import {
	type CopyValue,
	type FileSlot,
	type FormModel,
	type NavigateIntent,
	REQUEST_FILES_STORE_REF,
	type RunEntry,
	type RunOutcome,
	type SessionInput,
	type WorkbenchField,
} from "../contracts";
import { NO_PLACE_RESOURCE } from "../run/outcome";
import { EMPTY_RUN_SUMMARY } from "../run/summary";
import {
	FIXTURE_FIELDS,
	FIXTURE_HOSTED_HOST,
	fixtureForm,
} from "../testing/fixtures";
import {
	type RunHandle,
	type RuntimeEngine,
	type ScheduleFrame,
	answerInteraction,
	dispatchRun,
	dropStream,
	problemsOutcome,
	runPathOf,
	stopRun,
} from "./dispatch-run";
import type { InlineEncoder } from "./uploads";

// ─── Test parts ─────────────────────────────────────────────────────────────

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

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function deferred<T>() {
	let resolve: (value: T) => void = () => {};
	const promise = new Promise<T>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

const FLOW_PATH = {
	path: "tmp/global/apps/a/events/e/requests/r/0001-receipt.pdf",
	store_ref: REQUEST_FILES_STORE_REF,
	cache_store_ref: null,
};

const slot = (id: string, parts: Partial<FileSlot> = {}): FileSlot => ({
	id,
	name: `${id}.pdf`,
	size: 10,
	type: "application/pdf",
	state: "sent",
	progress: null,
	ref: { kind: "flowpath", flowPath: FLOW_PATH, url: null },
	error: null,
	sentAt: 1,
	expiresAt: null,
	...parts,
});

const runEntry = (
	values: Readonly<Record<string, CopyValue>>,
	parts: Partial<RunEntry> = {},
): RunEntry => ({
	id: "run-1",
	n: 1,
	origin: "session",
	status: "starting",
	createdAt: 1,
	startedAt: null,
	endedAt: null,
	copy: { values, presetName: null, leftAsIs: false, perRun: [], replaced: [] },
	target: "local",
	streamId: "run-1#1",
	backendRunId: null,
	output: null,
	outcome: null,
	summary: EMPTY_RUN_SUMMARY,
	stopRequested: false,
	pendingSlotIds: [],
	waitingForPlace: false,
	failedAt: null,
	unseenFailure: false,
	...parts,
});

type ExecuteOptions = Parameters<ExecutionEngineProvider["executeEvent"]>[1];

function engineWith(execute: ExecuteEventFn) {
	const engine = new ExecutionEngineProvider();
	engine.setBackend({} as unknown as IBackendState);
	engine.setExecuteEventFn(execute);
	const calls: { streamId: string; options: ExecuteOptions }[] = [];
	const spy: RuntimeEngine = {
		executeEvent: (streamId, options) => {
			calls.push({ streamId, options });
			return engine.executeEvent(streamId, options);
		},
		subscribeToEventStream: (...args) => engine.subscribeToEventStream(...args),
		unsubscribeFromEventStream: (...args) =>
			engine.unsubscribeFromEventStream(...args),
		hasStream: (streamId) => engine.hasStream(streamId),
		isStreamComplete: (streamId) => engine.isStreamComplete(streamId),
	};
	return { engine, spy, calls };
}

interface Harness {
	readonly handle: RunHandle;
	readonly reports: SessionInput[];
	readonly navigations: NavigateIntent[];
	readonly cancelled: string[];
	readonly streams: string[];
	readonly settled: Promise<RunOutcome>;
}

/** Frames that come only when the test says so. */
function manualFrames() {
	const due: (() => void)[] = [];
	const frame: ScheduleFrame = (callback) => {
		due.push(callback);
		return () => {
			const index = due.indexOf(callback);
			if (index >= 0) due.splice(index, 1);
		};
	};
	const next = () => {
		for (const callback of due.splice(0)) callback();
	};
	return { frame, next, waiting: () => due.length };
}

function send(parts: {
	readonly engine: RuntimeEngine;
	readonly form?: FormModel;
	readonly run?: RunEntry;
	readonly files?: ReadonlyMap<string, File>;
	readonly helper?: InlineEncoder;
	readonly frame?: ScheduleFrame;
}): Harness {
	const reports: SessionInput[] = [];
	const navigations: NavigateIntent[] = [];
	const cancelled: string[] = [];
	const streams: string[] = [];
	const done = deferred<RunOutcome>();
	const handle = dispatchRun({
		engine: parts.engine,
		eventState: {
			cancelExecution: async (runId) => {
				cancelled.push(runId);
			},
		},
		helper: parts.helper ?? { fileToUrl: async () => "data:," },
		form: parts.form ?? fixtureForm("none"),
		run: parts.run ?? runEntry({}),
		fileOf: (slotId) => parts.files?.get(slotId),
		report: (input) => reports.push(input),
		navigate: (intents) => navigations.push(...intents),
		onStream: (started) => streams.push(started.streamId),
		settle: (settled, outcome) => {
			reports.push({ type: "runSettled", runId: settled.runId, outcome });
			done.resolve(outcome);
		},
		frame: parts.frame,
	});
	return {
		handle,
		reports,
		navigations,
		cancelled,
		streams,
		settled: done.promise,
	};
}

const outputs = (reports: readonly SessionInput[]) =>
	reports.flatMap((input) =>
		input.type === "runOutput" ? [input.output] : [],
	);

// ─── Sending ────────────────────────────────────────────────────────────────

describe("dispatchRun", () => {
	test("the run sends its own copy as the payload of its node, on its own stream", async () => {
		const form = fixtureForm("small");
		let received: IRunPayload | undefined;
		const { spy, calls } = engineWith(async (_app, _event, payload) => {
			received = payload;
			return undefined;
		});
		const run = runEntry({ order: "A-1", quantity: "2", receipt: slot("r") });
		const harness = send({ engine: spy, form, run });
		await harness.settled;

		expect(received).toEqual({
			id: "node-return-request",
			payload: { order: "A-1", quantity: 2, receipt: FLOW_PATH },
		});
		expect(calls).toHaveLength(1);
		expect(calls[0].streamId).toBe("run-1#1");
		expect(calls[0].options).toMatchObject({
			appId: "app-shop-assistant",
			eventId: "event-return-request",
			streamState: false,
			path: "/use?id=app-shop-assistant&eventId=event-return-request",
			title: "Return request",
			interfaceType: "generic",
		});
		expect(harness.streams).toEqual(["run-1#1"]);
	});

	test("every event reaches the run once; a2ui navigation is taken once", async () => {
		const start = event("run_initiated", {});
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-1");
			cb?.([start, event("a2ui", { type: "navigateTo", route: "/review" })]);
			cb?.([start]);
			cb?.([
				event("generic_result", { total: 3 }),
				event("completed", { status: "completed" }),
			]);
			return { log_level: 1 } as ILogMetadata;
		});
		const harness = send({ engine: spy });
		const outcome = await harness.settled;

		expect(harness.reports[0]).toEqual({
			type: "runAccepted",
			runId: "run-1",
			backendRunId: "backend-1",
		});
		expect(outputs(harness.reports).map((output) => output.eventCount)).toEqual(
			[4],
		);
		expect(outputs(harness.reports).at(-1)?.result).toEqual({
			value: { total: 3 },
		});
		expect(harness.navigations).toEqual([{ route: "/review", replace: false }]);
		expect(harness.handle.accumulator.takeNavigation()).toEqual([]);
		expect(outcome).toEqual({ kind: "succeeded" });
	});

	test("a streamed answer reaches the session once a frame; the settle sends what is left first", async () => {
		const gate = deferred<void>();
		let chunk: (text: string) => void = () => {};
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-s");
			chunk = (text) => cb?.([event("chunk", { text })]);
			cb?.([event("run_initiated", {})]);
			await gate.promise;
			cb?.([event("completed", { status: "completed" })]);
			return undefined;
		});
		const frames = manualFrames();
		const harness = send({ engine: spy, frame: frames.frame });
		await flush();
		chunk("Ven");
		chunk("dor");
		expect(outputs(harness.reports)).toEqual([]);
		expect(frames.waiting()).toBe(1);
		frames.next();
		expect(outputs(harness.reports).map((output) => output.eventCount)).toEqual(
			[3],
		);
		chunk(": Nordwind");
		expect(frames.waiting()).toBe(1);
		gate.resolve();
		await harness.settled;
		expect(frames.waiting()).toBe(0);
		expect(harness.reports.slice(-2).map((input) => input.type)).toEqual([
			"runOutput",
			"runSettled",
		]);
		expect(outputs(harness.reports).map((output) => output.eventCount)).toEqual(
			[3, 5],
		);
		frames.next();
		expect(outputs(harness.reports)).toHaveLength(2);
	});

	test("an error event followed by completed failed settles as failed", async () => {
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-2");
			cb?.([event("run_initiated", {})]);
			cb?.([event("error", { message: "OCR failed on page 2" })]);
			cb?.([event("completed", { status: "failed", log_level: 4 })]);
			return undefined;
		});
		const outcome = await send({ engine: spy }).settled;
		expect(outcome).toMatchObject({
			kind: "failed",
			failure: "flow",
			message: "OCR failed on page 2",
		});
	});

	test("a desktop run that resolves cancelled after its events settles as stopped", async () => {
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-3");
			cb?.([
				event("run_initiated", {}),
				event("completed", { status: "cancelled" }),
			]);
			return { log_level: 4 } as ILogMetadata;
		});
		expect(await send({ engine: spy }).settled).toEqual({ kind: "stopped" });
	});

	test("an inline file goes as a data: URL; a hosted page gives no background link", async () => {
		const receipt = FIXTURE_FIELDS.small[2];
		const attachment: WorkbenchField = {
			...receipt,
			key: "attachment",
			name: "attachment",
			dataType: "PathBuf",
			fileMode: "url",
		};
		const form: FormModel = {
			...fixtureForm("small", FIXTURE_HOSTED_HOST),
			fields: [FIXTURE_FIELDS.small[0], attachment],
		};
		let received: IRunPayload | undefined;
		const { spy, calls } = engineWith(async (_a, _e, payload) => {
			received = payload;
			return undefined;
		});
		const run = runEntry(
			{ order: "A-1", attachment: slot("a", { ref: { kind: "inline" } }) },
			{ target: "remote" },
		);
		const files = new Map([["a", new File(["alpha"], "a.pdf")]]);
		const helper: InlineEncoder = {
			fileToUrl: async (file) =>
				`data:application/pdf;base64,${btoa(await file.text())}`,
		};
		await send({ engine: spy, form, run, files, helper }).settled;

		expect(received?.payload).toEqual({
			order: "A-1",
			attachment: `data:application/pdf;base64,${btoa("alpha")}`,
		});
		expect(calls[0].options.path).toBeUndefined();
	});

	test("a run whose file cannot be read or is still sending never starts", async () => {
		const receipt = FIXTURE_FIELDS.small[2];
		const attachment: WorkbenchField = {
			...receipt,
			key: "attachment",
			name: "attachment",
			fileMode: "url",
		};
		const hosted: FormModel = {
			...fixtureForm("small", FIXTURE_HOSTED_HOST),
			fields: [attachment],
		};
		const { spy, calls } = engineWith(async () => undefined);
		const unreadable = send({
			engine: spy,
			form: hosted,
			run: runEntry({ attachment: slot("gone", { ref: { kind: "inline" } }) }),
		});
		const sending = send({
			engine: spy,
			form: fixtureForm("small"),
			run: runEntry({
				order: "A-1",
				quantity: "1",
				receipt: slot("r", { state: "sending", ref: null }),
			}),
		});
		const fileNotSent: RunOutcome = {
			kind: "notStarted",
			reason: "fileNotSent",
		};
		expect(await unreadable.settled).toEqual(fileNotSent);
		expect(await sending.settled).toEqual(fileNotSent);
		expect(calls).toHaveLength(0);
		expect(unreadable.handle.dispatched).toBe(false);
	});
});

// ─── Stop ───────────────────────────────────────────────────────────────────

describe("stopRun", () => {
	test("Stop after the run id cancels the run on the host", async () => {
		const gate = deferred<void>();
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-7");
			cb?.([event("run_initiated", {})]);
			await gate.promise;
			return undefined;
		});
		const harness = send({ engine: spy });
		await flush();
		stopRun(harness.handle);
		expect(harness.cancelled).toEqual(["backend-7"]);
		gate.resolve();
		expect(await harness.settled).toEqual({ kind: "stopped" });
	});

	test("Stop before the run id cancels the run as soon as the id arrives", async () => {
		const started = deferred<(id: string) => void>();
		const { spy } = engineWith(
			(_a, _e, _p, _s, onEventId, cb) =>
				new Promise((resolve) => {
					started.resolve((id) => {
						onEventId?.(id);
						cb?.([event("run_initiated", {})]);
						resolve(undefined);
					});
				}),
		);
		const harness = send({ engine: spy });
		const begin = await started.promise;
		stopRun(harness.handle);
		expect(harness.cancelled).toEqual([]);
		begin("backend-8");
		expect(harness.cancelled).toEqual(["backend-8"]);
		expect(await harness.settled).toEqual({ kind: "stopped" });
	});

	test("Stop before the run was sent never sends it", async () => {
		const { spy, calls } = engineWith(async () => undefined);
		const harness = send({ engine: spy });
		stopRun(harness.handle);
		expect(await harness.settled).toEqual({
			kind: "notStarted",
			reason: "removedFromQueue",
		});
		expect(calls).toHaveLength(0);
		expect(harness.streams).toEqual([]);
	});
});

// ─── The quiet plan limit (spec 10.3) ───────────────────────────────────────

describe("a start refused for want of a place", () => {
	const globals = globalThis as { window?: unknown };
	const hadWindow = "window" in globals;
	const previousWindow = globals.window;
	let announced: string[] = [];

	const refusal = () =>
		apiResponseError(
			{ status: 402, statusText: "Payment Required", headers: new Headers() },
			JSON.stringify({
				error: {
					code: "PLAN_LIMIT_EXCEEDED",
					message: "Every cloud run of your plan is in use",
					quota: {
						resource: NO_PLACE_RESOURCE,
						scope: "account",
						payerId: "payer",
						plan: "FREE",
						used: 2,
						reserved: 0,
						limit: 2,
						unit: "executions",
					},
				},
			}),
			"apps/a/events/e/invoke",
		);

	beforeEach(() => {
		announced = [];
		const target = new EventTarget();
		target.addEventListener(PLAN_LIMIT_EVENT, (event) => {
			const { detail } = event as CustomEvent<ApiResponseError>;
			announced.push(detail.quota?.resource ?? "");
		});
		globals.window = Object.assign(target, { CustomEvent });
	});

	afterEach(() => {
		if (hadWindow) globals.window = previousWindow;
		else Reflect.deleteProperty(globals, "window");
	});

	test("keeps the upgrade dialog shut and settles as noPlace", async () => {
		const { spy } = engineWith(async () => {
			throw refusal();
		});
		const harness = send({
			engine: spy,
			run: runEntry({}, { target: "remote" }),
		});
		expect(await harness.settled).toEqual({ kind: "noPlace" });
		expect(announced).toEqual([]);

		refusal();
		expect(announced).toEqual([NO_PLACE_RESOURCE]);
	});

	test("the quiet ends when the run is accepted", async () => {
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId) => {
			onEventId?.("backend-9");
			refusal();
			return undefined;
		});
		await send({ engine: spy, run: runEntry({}, { target: "remote" }) })
			.settled;
		expect(announced).toEqual([NO_PLACE_RESOURCE]);
	});

	test("a run on this device does not hold it quiet", async () => {
		const { spy } = engineWith(async () => {
			throw refusal();
		});
		await send({ engine: spy, run: runEntry({}, { target: "local" }) }).settled;
		expect(announced).toEqual([NO_PLACE_RESOURCE]);
	});

	test("a run whose target is not known yet counts as the cloud", async () => {
		const { spy } = engineWith(async () => {
			throw refusal();
		});
		await send({ engine: spy, run: runEntry({}, { target: null }) }).settled;
		expect(announced).toEqual([]);
	});
});

// ─── Streams, questions, paths ──────────────────────────────────────────────

describe("dropStream", () => {
	test("a settled run's stream is deleted, so the background card never shows it", async () => {
		const { spy, engine } = engineWith(async (_a, _e, _p, _s, _id, cb) => {
			cb?.([event("completed", { status: "completed" })]);
			return undefined;
		});
		const harness = send({ engine: spy });
		await harness.settled;
		expect(engine.getBackgroundStreams().map((s) => s.streamId)).toEqual([
			"run-1#1",
		]);
		expect(dropStream(spy, "run-1#1", "form")).toBe(true);
		expect(engine.hasStream("run-1#1")).toBe(false);
		expect(engine.getBackgroundStreams()).toEqual([]);
	});

	test("a stream still running is kept", async () => {
		const gate = deferred<void>();
		const { spy, engine } = engineWith(async () => {
			await gate.promise;
			return undefined;
		});
		const harness = send({ engine: spy });
		await flush();
		expect(dropStream(spy, "run-1#1", "form")).toBe(false);
		expect(engine.hasStream("run-1#1")).toBe(true);
		gate.resolve();
		await harness.settled;
		expect(dropStream(spy, "unknown", "form")).toBe(true);
	});
});

describe("answerInteraction", () => {
	const question = {
		id: "q-1",
		name: "Line 4 has no purchase order",
		description: "Book it anyway?",
		interaction_type: { type: "single_choice", options: [] },
		status: "pending",
		ttl_seconds: 300,
		expires_at: 4_102_444_800,
	};

	test("an answer that went out moves the question to responded", async () => {
		const gate = deferred<void>();
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-10");
			cb?.([
				event("run_initiated", {}),
				event("interaction_request", question),
			]);
			await gate.promise;
			return undefined;
		});
		const harness = send({ engine: spy });
		await flush();
		const answered: [IInteractionRequest, unknown][] = [];
		const report: SessionInput[] = [];
		await answerInteraction(
			harness.handle,
			"q-1",
			"book",
			(input) => report.push(input),
			async (interaction, value) => {
				answered.push([interaction, value]);
			},
		);

		expect(
			answered.map(([interaction, value]) => [interaction.id, value]),
		).toEqual([["q-1", "book"]]);
		const last = outputs(report).at(-1);
		expect(last?.interactions).toMatchObject([
			{ id: "q-1", status: "responded", response_value: "book" },
		]);
		gate.resolve();
		await harness.settled;
	});

	test("an unknown question or a failed answer rejects; the question stays open", async () => {
		const gate = deferred<void>();
		const { spy } = engineWith(async (_a, _e, _p, _s, onEventId, cb) => {
			onEventId?.("backend-11");
			cb?.([event("interaction_request", question)]);
			await gate.promise;
			return undefined;
		});
		const harness = send({ engine: spy });
		await flush();
		await expect(
			answerInteraction(
				harness.handle,
				"q-2",
				"x",
				() => {},
				async () => {},
			),
		).rejects.toThrow("no open question q-2");
		await expect(
			answerInteraction(
				harness.handle,
				"q-1",
				"x",
				() => {},
				async () => {
					throw new Error("channel closed");
				},
			),
		).rejects.toThrow("channel closed");
		expect(harness.handle.accumulator.output().interactions[0].status).toBe(
			"pending",
		);
		gate.resolve();
		await harness.settled;
	});
});

describe("runPathOf and problemsOutcome", () => {
	test("the background card links to the app's /use page by route, else by event", () => {
		const form = fixtureForm("small");
		expect(runPathOf({ ...form, eventRoute: "/returns" })).toBe(
			"/use?id=app-shop-assistant&route=%2Freturns",
		);
		expect(runPathOf(form)).toBe(
			"/use?id=app-shop-assistant&eventId=event-return-request",
		);
		expect(
			runPathOf(fixtureForm("small", FIXTURE_HOSTED_HOST)),
		).toBeUndefined();
	});

	test("file problems mean the run was not sent; anything else failed", () => {
		expect(problemsOutcome({ receipt: { code: "pickAgain" } })).toEqual({
			kind: "notStarted",
			reason: "fileNotSent",
		});
		expect(problemsOutcome({ order: { code: "required" } })).toEqual({
			kind: "failed",
			failure: "other",
			message: null,
			detail: "The run's inputs could not be sent (order: required).",
		});
	});
});
