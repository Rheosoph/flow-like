import { describe, expect, test } from "bun:test";
import {
	FlowPilotCommandApplyError,
	executeFlowPilotCommandBatch,
	flowPilotCommandApplyDiagnostics,
	throwFlowPilotCommandApplyError,
} from "./flowpilot-command-apply";

describe("FlowPilot queued command apply failures", () => {
	test("refetches before preserving a concrete queue-element failure", async () => {
		let refetches = 0;
		const failure = throwFlowPilotCommandApplyError(
			{
				requestedCommands: 2,
				appliedCommands: 1,
				failures: [
					{
						queueIndex: 1,
						phase: "connection",
						commandType: "ConnectPins",
						message: 'Pin "result" was not found on node "Fetch"',
					},
				],
			},
			async () => {
				refetches += 1;
				return { data: { id: "fresh-board" }, error: null };
			},
		);

		await expect(failure).rejects.toBeInstanceOf(FlowPilotCommandApplyError);
		await expect(failure).rejects.toThrow(
			"1/2 queued commands confirmed applied",
		);
		await expect(failure).rejects.toThrow(
			"queue item 2 (connection), ConnectPins",
		);
		expect(refetches).toBe(1);
	});

	test("does not mask the apply diagnostic when board refetch also fails", async () => {
		let caught: unknown;
		try {
			await throwFlowPilotCommandApplyError(
				{
					requestedCommands: 101,
					appliedCommands: 100,
					failures: [
						{
							phase: "comment creation batch 2",
							commandType: "UpsertComment",
							message:
								"Command batch validation failed at index 0: stale comment",
						},
					],
				},
				async () => {
					throw { error: "canonical board fetch timed out" };
				},
			);
		} catch (error) {
			caught = error;
		}

		expect(caught).toBeInstanceOf(FlowPilotCommandApplyError);
		expect((caught as Error).message).toContain(
			"Command batch validation failed at index 0: stale comment",
		);
		expect((caught as Error).message).toContain(
			"Board refetch also failed: canonical board fetch timed out",
		);
		expect(flowPilotCommandApplyDiagnostics(caught)).toEqual([
			"comment creation batch 2, UpsertComment: Command batch validation failed at index 0: stale comment",
			"Board refetch failed: canonical board fetch timed out",
		]);
	});

	test("a later rejected batch refetches after retaining earlier progress", async () => {
		let executions = 0;
		let refetches = 0;
		const execute = async () => {
			executions += 1;
			if (executions === 1) return Array.from({ length: 100 }, (_, id) => id);
			throw {
				error:
					"Command batch validation failed at index 0: target node no longer exists",
			};
		};
		const refetch = async () => {
			refetches += 1;
			return { data: { id: "canonical" }, error: null };
		};

		const first = await executeFlowPilotCommandBatch<number>({
			requestedCommands: 101,
			alreadyAppliedCommands: 0,
			expectedBatchCommands: 100,
			phase: "comment creation batch 1",
			commandType: "UpsertComment",
			execute,
			refetch,
		});
		expect(first).toHaveLength(100);

		let caught: unknown;
		try {
			await executeFlowPilotCommandBatch<number>({
				requestedCommands: 101,
				alreadyAppliedCommands: first.length,
				expectedBatchCommands: 1,
				phase: "comment creation batch 2",
				commandType: "UpsertComment",
				execute,
				refetch,
			});
		} catch (error) {
			caught = error;
		}

		expect(refetches).toBe(1);
		expect(caught).toBeInstanceOf(FlowPilotCommandApplyError);
		expect((caught as FlowPilotCommandApplyError).appliedCommands).toBe(100);
		expect((caught as Error).message).toContain("target node no longer exists");
	});
});
