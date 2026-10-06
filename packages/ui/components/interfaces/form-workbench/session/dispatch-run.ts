/*
 * One run's trip through the execution engine (PLAN §3.3 step 5, §3.5, §3.7). Its inline files are
 * encoded, its copy becomes the payload, and `executeEvent` sends it on the run's own stream. Every
 * event reaches the run's accumulator once, from `onLiveEvents` (never from a subscription replay),
 * so a remount never reads an event twice and an a2ui navigateTo is taken exactly once. The outcome
 * is decided when the promise settles, from what the run sent and how the promise ended. While a
 * run in the cloud starts, a refusal for want of a place stays out of the upgrade dialog: the form
 * queues the run again itself.
 */
import { quietPlanLimit } from "../../../../lib/api-error";
import type { ExecutionEngineProvider } from "../../../../lib/execution-engine";
import { isRecord } from "../../../../lib/response-shape";
import type { IIntercomEvent } from "../../../../lib/schema/events/intercom-event";
import type { ILogMetadata } from "../../../../lib/schema/flow/log-metadata";
import type { IRunPayload } from "../../../../lib/schema/flow/run-payload";
import type { IInteractionRequest } from "../../../../lib/schema/interaction";
import type { IEventState } from "../../../../state/backend-state/event-state";
import { submitInteractionResponse } from "../../chat-default/respond-interaction";
import {
	type ILoadRun,
	adoptLoadRunId,
	cancelLoadRun,
	createLoadRun,
} from "../../page-load-run";
import type {
	FieldKey,
	FieldProblem,
	FieldProblemCode,
	FormModel,
	NavigateIntent,
	RunAccumulator,
	RunEntry,
	RunOutcome,
	RunSettlement,
	SessionInput,
} from "../contracts";
import { buildPayload } from "../model/payload";
import { createRunAccumulator } from "../run/events";
import { NO_PLACE_RESOURCE, outcomeOf } from "../run/outcome";
import { type InlineEncoder, encodeInlineValues } from "./uploads";

/** What the runtime needs of the execution engine (ExecutionEngineProvider). */
export type RuntimeEngine = Pick<
	ExecutionEngineProvider,
	| "executeEvent"
	| "subscribeToEventStream"
	| "unsubscribeFromEventStream"
	| "hasStream"
	| "isStreamComplete"
>;

export type RespondInteraction = (
	interaction: IInteractionRequest,
	value: unknown,
) => Promise<void>;

/** Calls `callback` about one frame from now; returns its cancel. */
export type ScheduleFrame = (callback: () => void) => () => void;

/** A streamed answer reaches the session at most once per frame, however many chunks arrive meanwhile. */
export const OUTPUT_FRAME_MS = 16;

const timerFrame: ScheduleFrame = (callback) => {
	const timer = setTimeout(callback, OUTPUT_FRAME_MS);
	return () => clearTimeout(timer);
};

/** One dispatched run, as long as its promise has not settled. */
export interface RunHandle {
	readonly runId: string;
	readonly streamId: string;
	readonly accumulator: RunAccumulator;
	readonly loadRun: ILoadRun;
	stopRequested: boolean;
	/** `executeEvent` was called: the engine has a stream for this run. */
	dispatched: boolean;
	accepted: boolean;
	settled: boolean;
	/** The cancel of the frame booked to report the run's newest output; null when nothing waits. */
	outputDue: (() => void) | null;
}

export interface RunDispatch {
	readonly engine: RuntimeEngine;
	readonly eventState: Pick<IEventState, "cancelExecution">;
	readonly helper: InlineEncoder;
	readonly form: FormModel;
	/** The run as the state holds it after it started (its stream id is set). */
	readonly run: RunEntry;
	readonly fileOf: (slotId: string) => File | undefined;
	/** `runAccepted` and `runOutput`. */
	readonly report: (input: SessionInput) => void;
	readonly navigate: (intents: readonly NavigateIntent[]) => void;
	/** `executeEvent` was called; the stream exists from now on. */
	readonly onStream: (handle: RunHandle) => void;
	/** The promise settled (or the run could not be sent): the outcome, decided once. */
	readonly settle: (handle: RunHandle, outcome: RunOutcome) => void;
	/** Books the next report of live output (default: an OUTPUT_FRAME_MS timer). */
	readonly frame?: ScheduleFrame;
}

const NO_RELEASE = () => {};
const NOOP_EVENTS = () => {};

const FILE_PROBLEMS: ReadonlySet<FieldProblemCode> = new Set<FieldProblemCode>([
	"fileSending",
	"fileFailed",
	"fileTooLarge",
	"fileNotHere",
	"pickAgain",
]);

const FILE_NOT_SENT: RunOutcome = { kind: "notStarted", reason: "fileNotSent" };

/** The background card's link back to the form: the app's `/use` page, never the host's pathname. */
export function runPathOf(
	form: Pick<FormModel, "appId" | "eventId" | "eventRoute" | "host">,
): string | undefined {
	if (form.host.kind !== "app") return undefined;
	const params = new URLSearchParams({ id: form.appId });
	if (form.eventRoute) params.set("route", form.eventRoute);
	else params.set("eventId", form.eventId);
	return `/use?${params.toString()}`;
}

/** A run that may go to the cloud: an unresolved target counts as the cloud (`capOf`). */
const inTheCloud = (run: Pick<RunEntry, "target">) => run.target !== "local";

/** A copy the reducer let through that still cannot be sent: files say "not sent", anything else failed. */
export function problemsOutcome(
	problems: Readonly<Record<FieldKey, FieldProblem>>,
): RunOutcome {
	const entries = Object.entries(problems);
	if (entries.some(([, problem]) => FILE_PROBLEMS.has(problem.code)))
		return FILE_NOT_SENT;
	const detail = entries
		.map(([key, problem]) => `${key}: ${problem.code}`)
		.join(", ");
	return {
		kind: "failed",
		failure: "other",
		message: null,
		detail: `The run's inputs could not be sent (${detail}).`,
	};
}

type Prepared =
	| { readonly ok: true; readonly body: IRunPayload }
	| { readonly ok: false; readonly outcome: RunOutcome };

async function prepare(input: RunDispatch): Promise<Prepared> {
	const { form, run } = input;
	let values: RunEntry["copy"]["values"];
	try {
		values = await encodeInlineValues(
			form.fields,
			run.copy.values,
			input.fileOf,
			input.helper,
		);
	} catch (error) {
		console.warn(`[form-workbench] run ${run.n} was not sent`, error);
		return { ok: false, outcome: FILE_NOT_SENT };
	}
	const built = buildPayload(form.fields, values);
	if (!built.ok) return { ok: false, outcome: problemsOutcome(built.problems) };
	return { ok: true, body: { id: form.nodeId, payload: built.payload } };
}

function accepted(
	input: RunDispatch,
	handle: RunHandle,
	backendRunId: string,
	release: () => void,
) {
	release();
	adoptLoadRunId(handle.loadRun, backendRunId);
	if (handle.settled || handle.accepted) return;
	handle.accepted = true;
	input.report({ type: "runAccepted", runId: handle.runId, backendRunId });
}

/** The run's newest output, now; a frame booked for it is no longer needed. */
function reportOutput(
	report: (input: SessionInput) => void,
	handle: RunHandle,
) {
	handle.outputDue?.();
	handle.outputDue = null;
	report({
		type: "runOutput",
		runId: handle.runId,
		output: handle.accumulator.output(),
	});
}

/**
 * Every batch reaches the accumulator at once and its navigation is taken at once; the output is reported on the next
 * frame, so a streamed answer renders the form once a frame instead of once a chunk.
 */
function received(
	input: RunDispatch,
	handle: RunHandle,
	events: readonly IIntercomEvent[],
) {
	if (handle.settled) return;
	handle.accumulator.push(events);
	handle.outputDue ??= (input.frame ?? timerFrame)(() => {
		handle.outputDue = null;
		if (!handle.settled) reportOutput(input.report, handle);
	});
	const intents = handle.accumulator.takeNavigation();
	if (intents.length > 0) input.navigate(intents);
}

/** Calls the engine; a synchronous throw becomes a rejection. */
function execute(
	input: RunDispatch,
	handle: RunHandle,
	body: IRunPayload,
	release: () => void,
): Promise<RunSettlement> {
	const { engine, form } = input;
	const sent = new Promise<unknown>((resolve) =>
		resolve(
			engine.executeEvent(handle.streamId, {
				appId: form.appId,
				eventId: form.eventId,
				payload: body,
				streamState: false,
				path: runPathOf(form),
				title: form.name || undefined,
				interfaceType: "generic",
				onExecutionStart: (id) => accepted(input, handle, id, release),
				onLiveEvents: (events) => received(input, handle, events),
			}),
		),
	);
	handle.dispatched = true;
	input.onStream(handle);
	return sent.then(
		(value): RunSettlement => ({
			kind: "resolved",
			meta: isRecord(value) ? (value as unknown as ILogMetadata) : null,
		}),
		(error): RunSettlement => ({ kind: "rejected", error }),
	);
}

const NOT_SENT: RunSettlement = { kind: "resolved", meta: null };

async function send(
	input: RunDispatch,
	handle: RunHandle,
	release: () => void,
): Promise<void> {
	const prepared = await prepare(input);
	if (!prepared.ok) return finish(input, handle, release, prepared.outcome);
	const settlement = handle.stopRequested
		? NOT_SENT
		: await execute(input, handle, prepared.body, release);
	const output = handle.accumulator.output();
	const outcome = outcomeOf({
		settlement,
		terminal: output.terminal,
		eventCount: output.eventCount,
		stopRequested: handle.stopRequested,
		host: input.form.host.kind,
	});
	finish(input, handle, release, outcome);
}

/** Output still waiting for its frame goes first, so the session never settles a run on stale output. */
function finish(
	input: RunDispatch,
	handle: RunHandle,
	release: () => void,
	outcome: RunOutcome,
) {
	release();
	if (handle.settled) return;
	if (handle.outputDue) reportOutput(input.report, handle);
	handle.settled = true;
	input.settle(handle, outcome);
}

/**
 * Sends a run that the reducer started (`dispatchRun`). Returns at once; everything else arrives
 * through the callbacks. A run in the cloud keeps `concurrent_cloud_executions` out of the upgrade
 * dialog until it is accepted or settles.
 */
export function dispatchRun(input: RunDispatch): RunHandle {
	const { run, form } = input;
	const handle: RunHandle = {
		runId: run.id,
		streamId: run.streamId ?? `${run.id}#1`,
		accumulator: createRunAccumulator({
			appId: form.appId,
			eventId: form.eventId,
		}),
		loadRun: createLoadRun(input.eventState),
		stopRequested: false,
		dispatched: false,
		accepted: false,
		settled: false,
		outputDue: null,
	};
	const release = inTheCloud(run)
		? quietPlanLimit(NO_PLACE_RESOURCE)
		: NO_RELEASE;
	send(input, handle, release).catch((error: unknown) => {
		console.error(`[form-workbench] run ${run.n} could not be sent`, error);
		finish(input, handle, release, {
			kind: "failed",
			failure: "other",
			message: null,
			detail: String(error),
		});
	});
	return handle;
}

/**
 * Stop (`stopRun`): the host cancels the run once its id is known (hosted links only stop
 * listening); a run whose id has not arrived is cancelled when it does, and one not sent yet is
 * never sent.
 */
export function stopRun(handle: RunHandle): void {
	handle.stopRequested = true;
	cancelLoadRun(handle.loadRun);
}

/** A no-op subscription: while the form holds one, the engine's background card leaves the stream out. */
export function holdStream(
	engine: RuntimeEngine,
	streamId: string,
	subscriberId: string,
): void {
	engine.subscribeToEventStream(streamId, subscriberId, NOOP_EVENTS);
}

export function releaseStream(
	engine: RuntimeEngine,
	streamId: string,
	subscriberId: string,
): void {
	engine.unsubscribeFromEventStream(streamId, subscriberId);
}

/**
 * After a run settled: subscribe and unsubscribe once, so the engine deletes the completed stream
 * (a completed stream nobody unsubscribes from stays in the background card as "Running"). True
 * when there is nothing left to drop: the stream is gone, or it is complete and someone else
 * still listens. False while the engine has not marked it complete yet.
 */
export function dropStream(
	engine: RuntimeEngine,
	streamId: string,
	subscriberId: string,
): boolean {
	if (!engine.hasStream(streamId)) return true;
	if (!engine.isStreamComplete(streamId)) return false;
	holdStream(engine, streamId, subscriberId);
	releaseStream(engine, streamId, subscriberId);
	return true;
}

let answerSeq = 0;

function answeredEvent(interactionId: string, value: unknown): IIntercomEvent {
	const now = Date.now();
	answerSeq += 1;
	return {
		event_id: `form-workbench-answer:${interactionId}:${now}:${answerSeq}`,
		event_type: "interaction_request",
		payload: { id: interactionId, status: "responded", response_value: value },
		timestamp: {
			secs_since_epoch: Math.floor(now / 1000),
			nanos_since_epoch: (now % 1000) * 1_000_000,
		},
	};
}

/**
 * Answers an in-run question on its channel. Once the answer went out, the run's own reading of the
 * question says "responded", so the run leaves `asking`. Rejects when the question is unknown or
 * the answer could not be sent; the question then stays open.
 */
export async function answerInteraction(
	handle: RunHandle,
	interactionId: string,
	value: unknown,
	report: (input: SessionInput) => void,
	respond: RespondInteraction = submitInteractionResponse,
): Promise<void> {
	const interaction = handle.accumulator
		.output()
		.interactions.find((item) => item.id === interactionId);
	if (!interaction)
		throw new Error(
			`Run ${handle.runId} has no open question ${interactionId} to answer.`,
		);
	await respond(interaction, value);
	if (handle.settled) return;
	handle.accumulator.push([answeredEvent(interactionId, value)]);
	reportOutput(report, handle);
}
