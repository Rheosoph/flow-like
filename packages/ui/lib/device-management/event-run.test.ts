import { expect, test } from "bun:test";
import {
	RUN_FAILURE_CODES,
	cancelRun,
	readRun,
	runEvent,
	runOpen,
	runPayloadProblem,
} from "./event-run";
import type { ManagementCall } from "./telemetry";
import type { ManagementResponse } from "./types";

/* The run_event literals of run-more-2-design §1.8; the agent's and the protocol's tests carry the same. */

const RUN_ID = "0b5f6a43-2a7e-4a52-9d38-2f1a4f0e9c11";
const ACCEPTED = JSON.parse(
	`{"operation_id":"op-1","state":"accepted",
 "result":{"command":"run_event","placement_id":"notes","event_id":"evt_notes_form","run_id":"${RUN_ID}","run":"queued"}}`,
);
const COMPLETED = JSON.parse(
	`{"operation_id":"op-1","state":"completed",
 "result":{"command":"run_event","placement_id":"notes","event_id":"evt_notes_form","run_id":"${RUN_ID}",
  "run":"succeeded","started_at":1790000000,"finished_at":1790000004,
  "output_bytes":9,"truncated":false,"attachments":0,"output":{"json":{"id":42}}}}`,
);
const FAILED = JSON.parse(
	`{"operation_id":"op-2","state":"failed",
 "result":{"command":"run_event","placement_id":"notes","event_id":"evt_notes_form","run_id":"${RUN_ID}",
  "run":"failed","code":"invalid_fields","fields":["title"]}}`,
);

function device(answers: ManagementResponse[]) {
	const sent: [Record<string, unknown>, string | undefined][] = [];
	const call: ManagementCall = async (command, operationId) => {
		sent.push([command, operationId]);
		const answer = answers.shift();
		if (!answer) throw new Error("no answer left");
		return answer;
	};
	return { call, sent };
}

const INPUT = {
	placementId: "notes",
	eventId: "evt_notes_form",
	expectedRevision: 7,
	payload: { title: "Hello", urgent: true },
};

test("a run is sent once with its operation id and answered at once with its run id", async () => {
	const { call, sent } = device([ACCEPTED]);
	expect(await runEvent(call, INPUT, "op-1")).toEqual({
		kind: "accepted",
		run: {
			operationId: "op-1",
			placementId: "notes",
			eventId: "evt_notes_form",
			runId: RUN_ID,
			run: "queued",
		},
	});
	expect(sent).toEqual([
		[
			{
				type: "run_event",
				placement_id: "notes",
				event_id: "evt_notes_form",
				expected_revision: 7,
				payload: { title: "Hello", urgent: true },
			},
			"op-1",
		],
	]);
	// A quick action sends no payload at all.
	const action = device([ACCEPTED]);
	await runEvent(action.call, { ...INPUT, payload: undefined }, "op-1");
	expect("payload" in action.sent[0][0]).toBe(false);
});

test("the operation read follows the run to its end, with the output only its issuer gets", async () => {
	const running = {
		...ACCEPTED,
		result: { ...ACCEPTED.result, run: "running", started_at: 1790000000 },
	};
	const { call, sent } = device([running, COMPLETED, FAILED]);
	const first = await readRun(call, "op-1");
	expect([first?.run, first?.startedAt, first && runOpen(first)]).toEqual([
		"running",
		1790000000,
		true,
	]);
	expect(await readRun(call, "op-1")).toEqual({
		operationId: "op-1",
		placementId: "notes",
		eventId: "evt_notes_form",
		runId: RUN_ID,
		run: "succeeded",
		startedAt: 1790000000,
		finishedAt: 1790000004,
		output: { json: { id: 42 } },
		outputBytes: 9,
		attachments: 0,
	});
	expect(sent[0]).toEqual([
		{ type: "operation", operation_id: "op-1" },
		undefined,
	]);
	expect(await readRun(call, "op-2")).toEqual({
		operationId: "op-2",
		placementId: "notes",
		eventId: "evt_notes_form",
		runId: RUN_ID,
		run: "failed",
		code: "invalid_fields",
		fields: ["title"],
	});
});

test("every failure code reads back; an unknown one keeps the run without a code", async () => {
	for (const code of RUN_FAILURE_CODES) {
		const run = code === "cancelled" || code === "timed_out" ? code : "failed";
		const { call } = device([
			{
				operation_id: "op-3",
				state: "failed",
				result: { ...FAILED.result, run, code, fields: undefined },
			},
		]);
		expect(await readRun(call, "op-3")).toMatchObject({ run, code });
	}
	const { call } = device([
		{
			operation_id: "op-3",
			state: "failed",
			result: { ...FAILED.result, code: "out_of_memory", fields: "title" },
		},
	]);
	const later = await readRun(call, "op-3");
	expect([later?.run, later?.code, later?.fields]).toEqual([
		"failed",
		undefined,
		undefined,
	]);
});

test("a cut or lost result says so; text output is kept as text", async () => {
	const { call } = device([
		{
			operation_id: "op-4",
			state: "completed",
			result: {
				...COMPLETED.result,
				output: { text: "Dear …" },
				truncated: true,
				output_bytes: 9000,
			},
		},
		{
			operation_id: "op-5",
			state: "completed",
			result: { ...COMPLETED.result, output: undefined, output_gone: true },
		},
	]);
	expect(await readRun(call, "op-4")).toMatchObject({
		output: { text: "Dear …" },
		truncated: true,
		outputBytes: 9000,
	});
	const gone = await readRun(call, "op-5");
	expect([gone?.outputGone, gone?.output]).toEqual([true, undefined]);
});

test("a run the journal does not know reads as none; a malformed run is an error", async () => {
	const unknown = device([
		{
			operation_id: "lookup",
			state: "rejected",
			result: { code: "invalid", error: "unknown operation", retryable: false },
		},
	]);
	expect(await readRun(unknown.call, "op-9")).toBeNull();
	const broken = device([
		{
			operation_id: "op-1",
			state: "completed",
			result: { ...COMPLETED.result, run: "paused" },
		},
	]);
	await expect(readRun(broken.call, "op-1")).rejects.toThrow(
		"invalid run op-1",
	);
	const other = device([
		{ operation_id: "lookup", state: "completed", result: {} },
	]);
	await expect(readRun(other.call, "op-1")).rejects.toThrow(
		"did not report run op-1",
	);
});

test("refusals come back with their code before anything is journaled", async () => {
	for (const code of [
		"unauthorized",
		"revision_conflict",
		"invalid",
		"busy",
		"limit",
		"unsupported",
	] as const) {
		const { call } = device([
			{
				operation_id: "op-1",
				state: "rejected",
				result: { code, error: `refused: ${code}`, retryable: code === "busy" },
			},
		]);
		expect(await runEvent(call, INPUT, "op-1")).toEqual({
			kind: "rejected",
			code,
			rejection: {
				code,
				error: `refused: ${code}`,
				retryable: code === "busy",
			},
		});
	}
	const { call } = device([
		{
			operation_id: "op-1",
			state: "rejected",
			result: { code: "quota", error: "later code" },
		},
	]);
	expect(await runEvent(call, INPUT, "op-1")).toMatchObject({
		kind: "rejected",
		code: "other",
	});
	const odd = device([{ ...ACCEPTED, operation_id: "op-other" }]);
	await expect(runEvent(odd.call, INPUT, "op-1")).rejects.toThrow(
		'state "accepted"',
	);
	const foreign = device([
		{ ...ACCEPTED, result: { ...ACCEPTED.result, event_id: "evt_other" } },
	]);
	await expect(runEvent(foreign.call, INPUT, "op-1")).rejects.toThrow(
		"another event's run",
	);
});

test("inputs over a device's bounds are refused before anything is sent", async () => {
	const many = Object.fromEntries(
		Array.from({ length: 65 }, (_, index) => [`f${index}`, index]),
	);
	const large = { text: "x".repeat(12_288) };
	const fits = { text: "x".repeat(12_277) };
	expect(runPayloadProblem(undefined)).toBeNull();
	expect(runPayloadProblem({})).toBeNull();
	expect(runPayloadProblem(many)).toBe("too_many");
	expect(runPayloadProblem(large)).toBe("too_large");
	expect(new TextEncoder().encode(JSON.stringify(fits)).length).toBe(12_288);
	expect(runPayloadProblem(fits)).toBeNull();
	expect(runPayloadProblem({ text: "ü".repeat(6_140) })).toBe("too_large");
	const { call, sent } = device([]);
	await expect(
		runEvent(call, { ...INPUT, payload: large }, "op-1"),
	).rejects.toThrow("at most 12288 bytes");
	await expect(
		runEvent(call, { ...INPUT, payload: many }, "op-1"),
	).rejects.toThrow("at most 64 fields");
	expect(sent).toEqual([]);
});

test("stopping a run says whether it was still going", async () => {
	const { call, sent } = device([
		{
			operation_id: "cancel-1",
			state: "completed",
			result: { command: "cancel_run", cancelled: true },
		},
		{
			operation_id: "cancel-2",
			state: "completed",
			result: { command: "cancel_run", cancelled: false },
		},
		{
			operation_id: "cancel-3",
			state: "rejected",
			result: { code: "unauthorized", error: "not your run" },
		},
	]);
	expect(await cancelRun(call, "op-1", "cancel-1")).toBe(true);
	expect(await cancelRun(call, "op-1", "cancel-2")).toBe(false);
	await expect(cancelRun(call, "op-1", "cancel-3")).rejects.toThrow(
		"not your run",
	);
	expect(sent[0]).toEqual([
		{ type: "cancel_run", operation_id: "op-1" },
		"cancel-1",
	]);
});
