import { z } from "zod";
import type { ManagementCall } from "./telemetry";
import {
	type CancelRunCommand,
	type ManagementRejection,
	type RunEventCommand,
	managementRejection,
} from "./types";

/*
 * A quick action or form run that a person starts on a device (design R2
 * §1.8). The device answers `run_event` at once with a run id; the outcome is
 * read with the journal's `operation` read. Inputs and output are never kept
 * by this module.
 */

export const RUN_PAYLOAD_MAX_KEYS = 64;
export const RUN_PAYLOAD_MAX_BYTES = 12_288;
/** A device sends at most this much output, measured as serialised JSON. */
export const RUN_OUTPUT_MAX_BYTES = 8192;

export const RUN_STATES = [
	"queued",
	"running",
	"succeeded",
	"failed",
	"cancelled",
	"timed_out",
] as const;
export type RunState = (typeof RUN_STATES)[number];
const OPEN_RUN_STATES: readonly RunState[] = ["queued", "running"];

/** Why a run did not succeed. */
export const RUN_FAILURE_CODES = [
	"flow_failed",
	"invalid_fields",
	"cancelled",
	"timed_out",
	"interrupted",
	"not_started",
	"needs_interaction",
] as const;
export type RunFailureCode = (typeof RUN_FAILURE_CODES)[number];

/** Refusals of `run_event` before anything is journaled; the sheet words each. */
export const RUN_REJECTIONS = [
	"unauthorized",
	"revision_conflict",
	"invalid",
	"busy",
	"limit",
	"unsupported",
] as const;
export type RunRejection = (typeof RUN_REJECTIONS)[number];

export type EventRunOutput = { json: unknown } | { text: string };

export interface EventRun {
	operationId: string;
	placementId: string;
	eventId: string;
	runId: string;
	run: RunState;
	/** Set when the run ended without success; absent for a code this client does not know. */
	code?: RunFailureCode;
	/** With `invalid_fields`: the names the device refused. */
	fields?: string[];
	/** Unix seconds by the device's clock. */
	startedAt?: number;
	finishedAt?: number;
	output?: EventRunOutput;
	/** The agent restarted since: the result is gone. */
	outputGone?: boolean;
	/** The output was cut to fit 8 KiB. */
	truncated?: boolean;
	outputBytes?: number;
	/** Media parts the run produced; never copied. */
	attachments?: number;
}

export interface RunEventInput {
	placementId: string;
	eventId: string;
	/** The placement's configuration revision the form was read for. */
	expectedRevision: number;
	/** Field name → value; absent for a quick action. */
	payload?: Record<string, unknown>;
}

export type RunEventAnswer =
	| { kind: "accepted"; run: EventRun }
	| {
			kind: "rejected";
			code: RunRejection | "other";
			rejection: ManagementRejection;
	  };

/** At most `max` characters as a device counts them: Unicode scalar values, so an emoji is one. */
export const deviceText = (max: number, min = 0) =>
	z
		.string()
		.min(min)
		.refine((value) => [...value].length <= max, `at most ${max} characters`);

const id = z.string().regex(/^[A-Za-z0-9_:.-]{1,128}$/u);
const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const outputSchema = z.union([
	z
		.object({ json: z.unknown() })
		.strict()
		.refine((value) => "json" in value),
	z.object({ text: z.string().max(RUN_OUTPUT_MAX_BYTES) }).strict(),
]);
const resultSchema = z.object({
	command: z.literal("run_event"),
	placement_id: id,
	event_id: id,
	run_id: z.string().uuid(),
	run: z.enum(RUN_STATES),
	code: z.enum(RUN_FAILURE_CODES).optional().catch(undefined),
	fields: z.array(deviceText(64)).max(16).optional().catch(undefined),
	started_at: count.optional(),
	finished_at: count.optional(),
	output_bytes: count.optional(),
	truncated: z.boolean().optional(),
	attachments: count.optional(),
	output: outputSchema.optional().catch(undefined),
	output_gone: z.boolean().optional(),
});

/** Drops the facts the device did not send. */
function present<T extends object>(record: T): T {
	return Object.fromEntries(
		Object.entries(record).filter(([, value]) => value !== undefined),
	) as T;
}

function eventRun(operationId: string, result: unknown): EventRun {
	const row = resultSchema.parse(result);
	return present<EventRun>({
		operationId,
		placementId: row.placement_id,
		eventId: row.event_id,
		runId: row.run_id,
		run: row.run,
		code: runOpen(row) ? undefined : row.code,
		fields: row.fields?.length ? row.fields : undefined,
		startedAt: row.started_at,
		finishedAt: row.finished_at,
		output: row.output as EventRunOutput | undefined,
		outputGone: row.output_gone || undefined,
		truncated: row.truncated || undefined,
		outputBytes: row.output_bytes,
		attachments: row.attachments,
	});
}

/** Whether the run still goes on. */
export function runOpen(run: Pick<EventRun, "run">): boolean {
	return OPEN_RUN_STATES.includes(run.run);
}

/** Checked before sending: a device refuses inputs over its bounds as `invalid`. */
export function runPayloadProblem(
	payload: Record<string, unknown> | undefined,
): "too_large" | "too_many" | null {
	if (!payload) return null;
	if (Object.keys(payload).length > RUN_PAYLOAD_MAX_KEYS) return "too_many";
	return new TextEncoder().encode(JSON.stringify(payload)).length >
		RUN_PAYLOAD_MAX_BYTES
		? "too_large"
		: null;
}

function rejectionOf(rejection: ManagementRejection): RunEventAnswer {
	const code = (RUN_REJECTIONS as readonly string[]).includes(rejection.code)
		? (rejection.code as RunRejection)
		: "other";
	return { kind: "rejected", code, rejection };
}

/**
 * Starts one run. Send it with an operation id kept for the whole attempt: a
 * repeated send answers the stored run and never starts a second one. Only an
 * agent with `on_demand_events` may be asked.
 */
export async function runEvent(
	call: ManagementCall,
	input: RunEventInput,
	operationId: string,
): Promise<RunEventAnswer> {
	const problem = runPayloadProblem(input.payload);
	if (problem)
		throw new RangeError(
			problem === "too_many"
				? `A run takes at most ${RUN_PAYLOAD_MAX_KEYS} fields.`
				: `A run's inputs take at most ${RUN_PAYLOAD_MAX_BYTES} bytes.`,
		);
	const command: RunEventCommand = {
		type: "run_event",
		placement_id: input.placementId,
		event_id: input.eventId,
		expected_revision: input.expectedRevision,
		...(input.payload ? { payload: input.payload } : {}),
	};
	const response = await call({ ...command }, operationId);
	const rejection = managementRejection(response);
	if (rejection) return rejectionOf(rejection);
	if (
		response.operation_id !== operationId ||
		!["accepted", "completed", "failed"].includes(response.state)
	)
		throw new Error(
			`The device answered the run of ${input.eventId} with state "${response.state}" instead of a run. Check the run again.`,
		);
	const run = eventRun(operationId, response.result);
	if (run.placementId !== input.placementId || run.eventId !== input.eventId)
		throw new Error(
			`The device answered the run of ${input.eventId} with another event's run.`,
		);
	return { kind: "accepted", run };
}

/** Reads a run by its operation id; null when the device's journal does not know it (any more). */
export async function readRun(
	call: ManagementCall,
	operationId: string,
): Promise<EventRun | null> {
	const response = await call({
		type: "operation",
		operation_id: operationId,
	});
	if (response.operation_id !== operationId) {
		if (managementRejection(response)?.code === "invalid") return null;
		throw new Error(
			`The device did not report run ${operationId} (state ${response.state}).`,
		);
	}
	if (response.state === "rejected") return null;
	try {
		return eventRun(operationId, response.result);
	} catch (error) {
		throw new Error(
			`The device returned an invalid run ${operationId}: ${error instanceof Error ? error.message : String(error)}`,
		);
	}
}

/**
 * Asks the device to stop a run; only its issuer or the device's owner may.
 * False when the run had already ended.
 */
export async function cancelRun(
	call: ManagementCall,
	operationId: string,
	cancelOperationId?: string,
): Promise<boolean> {
	const command: CancelRunCommand = {
		type: "cancel_run",
		operation_id: operationId,
	};
	const response = await call({ ...command }, cancelOperationId);
	const rejection = managementRejection(response);
	if (rejection)
		throw new Error(
			`The device refused to stop run ${operationId}: ${rejection.error}`,
		);
	if (response.state !== "completed")
		throw new Error(
			`The device answered "${response.state}" when asked to stop run ${operationId}.`,
		);
	const parsed = z
		.object({ command: z.literal("cancel_run"), cancelled: z.boolean() })
		.safeParse(response.result);
	if (!parsed.success)
		throw new Error(
			`The device's answer to stopping run ${operationId} is invalid.`,
		);
	return parsed.data.cancelled;
}
