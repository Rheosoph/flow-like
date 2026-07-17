import {
	createTableRuntime,
	executeNodeRuntime,
	queryExecutionLogsRuntime,
} from "@flow-like/flow-like-ui/hooks/use-frontend-runtime-tool-executor";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import {
	getPendingDatabaseSchemas,
	resetDatabaseCapabilitySessionForTests,
	shouldSkipUnavailableCreateTableApproval,
} from "@flow-like/flow-like-ui/lib/database-capability-session";
import { beforeEach, describe, expect, test, vi } from "vitest";

const metadata = {
	app_id: "app-1",
	board_id: "board-1",
	run_id: "run-1",
	event_id: "",
	node_id: "node-1",
	start: 1,
	end: 2,
	log_level: 1,
	payload: [],
	version: "0.0.1",
};

describe("FlowPilot frontend runtime execution", () => {
	beforeEach(() => {
		resetDatabaseCapabilitySessionForTests();
	});

	test("degrades a stale API create-table 405 without aborting the board build", async () => {
		const fields = [{ name: "ticket_id", type: "string", nullable: false }];
		const createTable = vi.fn(async () => {
			throw new ApiResponseError({
				status: 405,
				statusText: "Method Not Allowed",
				message: "Method Not Allowed",
			});
		});

		const result = await createTableRuntime({ createTable } as never, {
			appId: "app-1",
			tableName: "support_requests",
			fields,
			ifNotExists: true,
			userScoped: false,
		});

		expect(result).toMatchObject({
			status: "partial",
			code: "explicit_schema_create_not_deployed",
			created: false,
			next_action: "continue_workflow_build",
			requested_fields: fields,
			pending_schema_count: 1,
			network_request_skipped: false,
		});
	});

	test("latches an undeployed create-table endpoint and retains later schemas without network or approval", async () => {
		const createTable = vi.fn(async () => {
			throw new ApiResponseError({
				status: 405,
				statusText: "Method Not Allowed",
				message: "Method Not Allowed",
			});
		});
		const firstFields = [
			{ name: "ticket_id", type: "string", nullable: false },
		];
		const secondFields = [
			{ name: "history_id", type: "string", nullable: false },
		];

		await createTableRuntime({ createTable } as never, {
			appId: "app-1",
			tableName: "support_requests",
			fields: firstFields,
			ifNotExists: true,
			userScoped: false,
		});
		const cachedResult = await createTableRuntime({ createTable } as never, {
			appId: "app-1",
			tableName: "support_history",
			fields: secondFields,
			ifNotExists: true,
			userScoped: false,
		});

		expect(createTable).toHaveBeenCalledTimes(1);
		expect(cachedResult).toMatchObject({
			status: "partial",
			code: "explicit_schema_create_not_deployed",
			table_name: "support_history",
			requested_fields: secondFields,
			pending_schema_count: 2,
			network_request_skipped: true,
		});
		expect(getPendingDatabaseSchemas()).toMatchObject([
			{ tableName: "support_requests", fields: firstFields },
			{ tableName: "support_history", fields: secondFields },
		]);
		expect(
			shouldSkipUnavailableCreateTableApproval("database_tool", {
				operation: "create_table",
				app_id: "app-1",
			}),
		).toBe(true);
		expect(
			shouldSkipUnavailableCreateTableApproval("database_tool", {
				operation: "insert",
			}),
		).toBe(false);
		expect(
			shouldSkipUnavailableCreateTableApproval("database_tool", {
				operation: "create_table",
				app_id: "different-app",
			}),
		).toBe(false);

		const otherAppResult = await createTableRuntime({ createTable } as never, {
			appId: "different-app",
			tableName: "support_requests",
			fields: firstFields,
			ifNotExists: true,
			userScoped: false,
		});
		expect(createTable).toHaveBeenCalledTimes(2);
		expect(otherAppResult).toMatchObject({
			pending_schema_count: 1,
			network_request_skipped: false,
		});
	});

	test("executes a real board node and returns bounded live events", async () => {
		const getBoard = vi.fn(async () => ({
			nodes: {
				"node-1": {
					id: "node-1",
					name: "http_request",
					friendly_name: "HTTP Request",
				},
			},
		}));
		const executeBoard = vi.fn(
			async (
				_appId: string,
				_boardId: string,
				_payload: unknown,
				_streamState: boolean,
				onRunId: (id: string) => void,
				onEvents: (events: unknown[]) => void,
			) => {
				onRunId("run-from-stream");
				onEvents([
					{ event_type: "node_started", payload: { node_id: "node-1" } },
					{ event_type: "completed", payload: { ok: true } },
				]);
				return metadata;
			},
		);

		const result = await executeNodeRuntime(
			{ getBoard } as never,
			executeBoard as never,
			{
				appId: "app-1",
				boardId: "board-1",
				nodeId: "node-1",
				payload: { ticket_id: "T-42" },
			},
		);

		expect(getBoard).toHaveBeenCalledWith("app-1", "board-1", undefined, true);
		expect(executeBoard.mock.calls[0]?.[2]).toEqual({
			id: "node-1",
			payload: { ticket_id: "T-42" },
		});
		expect(result).toMatchObject({
			status: "ok",
			run_id: "run-1",
			node_name: "HTTP Request",
			live_event_count: 2,
		});
		expect(result.live_events).toEqual([
			{ event_type: "node_started", payload: { node_id: "node-1" } },
			{ event_type: "completed", payload: { ok: true } },
		]);
	});

	test("refuses to execute a stale or invented node id", async () => {
		const executeBoard = vi.fn();
		await expect(
			executeNodeRuntime(
				{ getBoard: vi.fn(async () => ({ nodes: {} })) } as never,
				executeBoard as never,
				{
					appId: "app-1",
					boardId: "board-1",
					nodeId: "missing",
				},
			),
		).rejects.toThrow("Node 'missing' was not found");
		expect(executeBoard).not.toHaveBeenCalled();
	});

	test("reads persisted logs directly from returned run metadata", async () => {
		const listRuns = vi.fn();
		const queryRun = vi.fn(async () => [
			{
				node_id: "node-1",
				log_level: "Info",
				message: "support response drafted",
				start: { secs_since_epoch: 1, nanos_since_epoch: 0 },
				end: { secs_since_epoch: 2, nanos_since_epoch: 0 },
			},
		]);

		const result = await queryExecutionLogsRuntime(
			{ listRuns, queryRun } as never,
			{
				appId: "app-1",
				boardId: "board-1",
				runId: "run-1",
				runMetadata: metadata,
				filter: "log_level >= 2",
				limit: 500,
			},
		);

		expect(listRuns).not.toHaveBeenCalled();
		expect(queryRun).toHaveBeenCalledWith(metadata, "log_level >= 2", 0, 100);
		expect(result).toMatchObject({
			status: "ok",
			run_id: "run-1",
			limit: 100,
			log_count: 1,
			verification: {
				complete: true,
				warning_count: 0,
				error_count: 0,
				fatal_count: 0,
				has_errors: false,
			},
			logs: [
				{
					node_id: "node-1",
					log_level: "Info",
					message: "support response drafted",
				},
			],
		});
	});

	test("resolves metadata by run id before querying logs", async () => {
		const listRuns = vi.fn(async () => [metadata]);
		const queryRun = vi.fn(async () => []);

		await queryExecutionLogsRuntime({ listRuns, queryRun } as never, {
			appId: "app-1",
			boardId: "board-1",
			runId: "run-1",
		});

		expect(listRuns).toHaveBeenCalledWith(
			"app-1",
			"board-1",
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			0,
			100,
			false,
		);
		expect(queryRun).toHaveBeenCalledWith(metadata, "", 0, 100);
	});

	test("reports runtime failures separately from successful tool transport", async () => {
		const log = (log_level: string, message: string) => ({
			node_id: "node-1",
			log_level,
			message,
			start: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			end: { secs_since_epoch: 2, nanos_since_epoch: 0 },
		});
		const result = await queryExecutionLogsRuntime(
			{
				listRuns: vi.fn(),
				queryRun: vi.fn(async () => [
					log("Warn", "slow response"),
					log("Error", "SMTP failed"),
					log("Fatal", "run aborted"),
				]),
			} as never,
			{
				appId: "app-1",
				boardId: "board-1",
				runId: "run-1",
				runMetadata: metadata,
			},
		);

		expect(result.status).toBe("ok");
		expect(result.verification).toEqual({
			scope: "returned_page",
			complete: true,
			warning_count: 1,
			error_count: 1,
			fatal_count: 1,
			has_errors: true,
		});
	});
});
