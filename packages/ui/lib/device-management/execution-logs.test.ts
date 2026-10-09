import { describe, expect, test } from "bun:test";
import { readExecutionLogs, readExecutionRuns } from "./execution-logs";
import type { ManagementCall } from "./telemetry";
import type { ManagementResponse } from "./types";
import { LiveCallError } from "./workspace/errors";

const FEATURES = { execution_logs: 1 } as const;
const PLACEMENT = "invoice-worker";
const RUN = "019abcde-1234-7000-8000-123456789abc";
const START = 1_791_541_800_123_456;
const END = START + 123_456;

function run(patch: Record<string, unknown> = {}) {
	return {
		run_id: RUN,
		board_id: "invoice-flow",
		event_id: "invoice-event",
		node_id: "start-node",
		version: "v1-2-3",
		event_version: "1.0.0",
		start: START,
		end: END,
		log_level: 1,
		logs: 7,
		...patch,
	};
}

function log(patch: Record<string, unknown> = {}) {
	return {
		message: "Invoice parsed",
		node_id: "parse-node",
		operation_id: "parse:1",
		log_level: 2,
		start: START,
		end: END,
		truncated: false,
		...patch,
	};
}

function runsPage(patch: Record<string, unknown> = {}) {
	return {
		placement_id: PLACEMENT,
		runs: [run()],
		next_offset: null,
		...patch,
	};
}

function logsPage(patch: Record<string, unknown> = {}) {
	return {
		placement_id: PLACEMENT,
		run_id: RUN,
		logs: [log()],
		next_offset: null,
		...patch,
	};
}

function device(result: Record<string, unknown>, state = "completed") {
	const sent: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command) => {
		sent.push(command);
		return { operation_id: "read-1", state, result } as ManagementResponse;
	};
	return { call, sent };
}

const READS = [
	{
		name: "runs",
		read: readExecutionRuns,
		input: { placementId: PLACEMENT, runId: RUN },
		page: runsPage,
		maximum: 20,
	},
	{
		name: "logs",
		read: readExecutionLogs,
		input: { placementId: PLACEMENT, runId: RUN },
		page: logsPage,
		maximum: 50,
	},
] as const;

describe("device workflow execution reads", () => {
	for (const { name, read, input, page, maximum } of READS) {
		test(`${name}: older agents are not asked for execution logs`, async () => {
			const { call, sent } = device({});
			for (const features of [undefined, {}]) {
				expect(await read(call, features, input)).toEqual({
					kind: "unsupported",
					feature: "execution_logs",
				});
			}
			expect(sent).toHaveLength(0);
		});

		test(`${name}: an agent's unsupported refusal remains distinguishable from empty history`, async () => {
			const { call } = device(
				{ code: "unsupported", error: "Update the agent.", retryable: false },
				"rejected",
			);
			expect(await read(call, FEATURES, input)).toEqual({
				kind: "unsupported",
				feature: "execution_logs",
			});
		});

		test(`${name}: authorization refusal reaches the caller`, async () => {
			const { call } = device(
				{
					code: "unauthorized",
					error: "Logs access was revoked.",
					retryable: false,
				},
				"rejected",
			);
			const failure = await read(call, FEATURES, input).catch(
				(error: unknown) => error,
			);
			expect(failure).toBeInstanceOf(LiveCallError);
			expect(failure).toMatchObject({
				code: "rejected_unauthorized",
				message: "Logs access was revoked.",
			});
		});

		test(`${name}: a response from another service is rejected`, async () => {
			const { call } = device(page({ placement_id: "other-service" }));
			await expect(read(call, FEATURES, input)).rejects.toThrow(
				"invalid workflow",
			);
		});

		test(`${name}: an accepted operation is not treated as completed history`, async () => {
			const { call } = device(page(), "accepted");
			await expect(read(call, FEATURES, input)).rejects.toThrow(
				'state "accepted"',
			);
		});

		test(`${name}: page bounds are checked before sending`, () => {
			const { call, sent } = device(page());
			for (const offset of [-1, 0.5, 1_000_001, Number.NaN]) {
				expect(() => read(call, FEATURES, { ...input, offset })).toThrow();
			}
			for (const limit of [0, -1, 1.5, maximum + 1, Number.POSITIVE_INFINITY]) {
				expect(() => read(call, FEATURES, { ...input, limit })).toThrow();
			}
			expect(sent).toHaveLength(0);
		});

		test(`${name}: only a strictly increasing bounded cursor is accepted`, async () => {
			for (const next_offset of [0, 9, 10, 10.5, 1_000_001]) {
				const { call } = device(page({ next_offset }));
				await expect(
					read(call, FEATURES, { ...input, offset: 10 }),
				).rejects.toThrow("next_offset");
			}
			for (const next_offset of [11, 1_000_000, null]) {
				const { call } = device(page({ next_offset }));
				expect(
					await read(call, FEATURES, { ...input, offset: 10 }),
				).toMatchObject({
					kind: "ok",
					data: { next_offset },
				});
			}
		});

		test(`${name}: the paging boundary preserves remaining-history status`, async () => {
			const { call } = device(page({ limit_reached: true }));
			expect(
				await read(call, FEATURES, { ...input, offset: 1_000_000 }),
			).toMatchObject({
				kind: "ok",
				data: { next_offset: null, limit_reached: true },
			});
			const malformed = device(page({ limit_reached: "true" }));
			await expect(read(malformed.call, FEATURES, input)).rejects.toThrow(
				"limit_reached",
			);
		});
	}

	test("run summaries keep microsecond timestamps, version strings and nullable log counts", async () => {
		const result = runsPage({
			runs: [run(), run({ logs: null, event_version: null })],
		});
		const { call, sent } = device(result);
		expect(
			await readExecutionRuns(call, FEATURES, { placementId: PLACEMENT }),
		).toEqual({
			kind: "ok",
			data: result,
		});
		expect(sent).toEqual([
			{ type: "execution_runs", placement_id: PLACEMENT, offset: 0, limit: 20 },
		]);
	});

	test("log reads preserve messages and Unix microseconds without coercion", async () => {
		const message = '<script>alert("log text")</script>\nsecond line';
		const result = logsPage({
			logs: [
				log({ message, node_id: null, operation_id: null, truncated: true }),
			],
		});
		const { call, sent } = device(result);
		expect(
			await readExecutionLogs(call, FEATURES, {
				placementId: PLACEMENT,
				runId: RUN,
			}),
		).toEqual({
			kind: "ok",
			data: result,
		});
		expect(sent).toEqual([
			{
				type: "execution_logs",
				placement_id: PLACEMENT,
				run_id: RUN,
				offset: 0,
				limit: 50,
			},
		]);
	});

	test("empty history is a successful result", async () => {
		const { call } = device(runsPage({ runs: [] }));
		expect(
			await readExecutionRuns(call, FEATURES, { placementId: PLACEMENT }),
		).toEqual({
			kind: "ok",
			data: runsPage({ runs: [] }),
		});
	});

	test("a log page for another run is rejected", async () => {
		const { call } = device(logsPage({ run_id: "another-run" }));
		await expect(
			readExecutionLogs(call, FEATURES, { placementId: PLACEMENT, runId: RUN }),
		).rejects.toThrow("run_id");
	});

	test("node and minimum-level filters are sent with each paginated read, including level zero", async () => {
		const { call, sent } = device(logsPage());
		for (const minLevel of [0, 4]) {
			await readExecutionLogs(call, FEATURES, {
				placementId: PLACEMENT,
				runId: RUN,
				nodeId: "parse:node-1",
				minLevel,
				offset: 25,
				limit: 10,
			});
		}
		expect(sent).toEqual(
			[0, 4].map((min_level) => ({
				type: "execution_logs",
				placement_id: PLACEMENT,
				run_id: RUN,
				node_id: "parse:node-1",
				min_level,
				offset: 25,
				limit: 10,
			})),
		);
	});

	test("invalid identifiers and levels never reach the device", () => {
		const { call, sent } = device({});
		for (const placementId of [
			"",
			"../other-service",
			"x".repeat(129),
			"service\u0000",
		]) {
			expect(() =>
				readExecutionRuns(call, FEATURES, { placementId }),
			).toThrow();
		}
		for (const patch of [
			{ runId: "../other-run" },
			{ nodeId: "node' OR true" },
			{ minLevel: -1 },
			{ minLevel: 5 },
			{ minLevel: 1.5 },
		]) {
			expect(() =>
				readExecutionLogs(call, FEATURES, {
					placementId: PLACEMENT,
					runId: RUN,
					...patch,
				}),
			).toThrow();
		}
		expect(sent).toHaveLength(0);
	});

	test("responses cannot exceed the requested page size", async () => {
		const runs = device(runsPage({ runs: [run(), run()] }));
		await expect(
			readExecutionRuns(runs.call, FEATURES, {
				placementId: PLACEMENT,
				limit: 1,
			}),
		).rejects.toThrow("runs");
		const logs = device(logsPage({ logs: [log(), log()] }));
		await expect(
			readExecutionLogs(logs.call, FEATURES, {
				placementId: PLACEMENT,
				runId: RUN,
				limit: 1,
			}),
		).rejects.toThrow("logs");
	});

	test("malformed or oversized run metadata is refused", async () => {
		for (const patch of [
			{ run_id: "../outside" },
			{ version: [1, 2, 3] },
			{ board_id: "x".repeat(129) },
			{ logs: "7" },
			{ logs: -1 },
			{ start: Number.MAX_SAFE_INTEGER + 1 },
			{ end: -1 },
		]) {
			const { call } = device(runsPage({ runs: [run(patch)] }));
			await expect(
				readExecutionRuns(call, FEATURES, { placementId: PLACEMENT }),
			).rejects.toThrow("invalid workflow executions");
		}
	});

	test("malformed and oversized log values are refused without coercion", async () => {
		for (const patch of [
			{ message: "x".repeat(8193) },
			{ message: { html: "<script>untrusted</script>" } },
			{ node_id: "x".repeat(129) },
			{ log_level: 5 },
			{ start: String(START) },
			{ end: Number.MAX_SAFE_INTEGER + 1 },
			{ truncated: "false" },
		]) {
			const { call } = device(logsPage({ logs: [log(patch)] }));
			await expect(
				readExecutionLogs(call, FEATURES, {
					placementId: PLACEMENT,
					runId: RUN,
				}),
			).rejects.toThrow("invalid workflow execution logs");
		}
	});
});
