import { describe, expect, test } from "bun:test";

import { normalizeDatabaseTableIdentifier } from "../lib/database-table-name";
import {
	executeNodeRuntime,
	resolveUiInspectWidgetEntries,
	runBoardTestsRuntime,
} from "./use-frontend-runtime-tool-executor";

describe("normalizeDatabaseTableIdentifier", () => {
	test("keeps valid physical identifiers unchanged", () => {
		expect(normalizeDatabaseTableIdentifier("Existing.Table-v2")).toBe(
			"Existing.Table-v2",
		);
	});

	test("maps human-facing labels to stable semantic identifiers", () => {
		expect(normalizeDatabaseTableIdentifier("Library Files")).toBe(
			"library_files",
		);
		expect(normalizeDatabaseTableIdentifier("R&D / Reports")).toBe(
			"r_and_d_reports",
		);
	});
});

describe("resolveUiInspectWidgetEntries", () => {
	const list = [
		[
			"app-expenses",
			"widget-expense-row",
			{ name: "Expense Row", description: "Reusable expense item." },
		],
		[
			"app-expenses",
			"widget-legacy",
			{ title: "Legacy Row", description: "Legacy metadata title." },
		],
		["app-expenses", "widget-no-metadata", undefined],
	] as const;

	test("normalizes the [appId, widgetId, metadata] contract", () => {
		expect(resolveUiInspectWidgetEntries(list).entries).toEqual([
			{
				widgetId: "widget-expense-row",
				selector: "Expense Row",
				description: "Reusable expense item.",
			},
			{
				widgetId: "widget-legacy",
				selector: "Legacy Row",
				description: "Legacy metadata title.",
			},
			{
				widgetId: "widget-no-metadata",
				selector: "widget-no-metadata",
				description: undefined,
			},
		]);
	});

	test("resolves selectors by widget id or real metadata name", () => {
		expect(
			resolveUiInspectWidgetEntries(list, "widget-expense-row").match,
		).toMatchObject({
			widgetId: "widget-expense-row",
			selector: "Expense Row",
		});
		expect(
			resolveUiInspectWidgetEntries(list, "Expense Row").match,
		).toMatchObject({
			widgetId: "widget-expense-row",
			selector: "Expense Row",
		});
		expect(
			resolveUiInspectWidgetEntries(list, "Legacy Row").match,
		).toMatchObject({
			widgetId: "widget-legacy",
			selector: "Legacy Row",
		});
	});

	test("never treats the tuple app id as a widget selector", () => {
		expect(
			resolveUiInspectWidgetEntries(list, "app-expenses").match,
		).toBeUndefined();
	});
});

/**
 * A published app's users hold execute permission without board read — the
 * board is neither on the device nor fetchable. The node run still has to
 * happen: the executor escalates it to the server, which resolves the node.
 */
describe("executeNodeRuntime", () => {
	const args = { appId: "app-1", boardId: "board-1", nodeId: "node-1" };

	test("runs the node even when the board cannot be read", async () => {
		const executeBoard = async (
			_appId: string,
			_boardId: string,
			_payload: unknown,
			_streamState: boolean,
			onId?: (id: string) => void,
		) => {
			onId?.("run-1");
			return undefined;
		};

		const result = await executeNodeRuntime(
			{ getBoard: () => Promise.reject(new Error("forbidden")) } as never,
			executeBoard as never,
			args,
		);

		expect(result.status).toBe("ok");
		expect(result.run_id).toBe("run-1");
		expect(result.node_name).toBeUndefined();
	});

	test("still rejects an unknown node on a readable board", async () => {
		const executeBoard = async () => {
			throw new Error("must not execute");
		};

		await expect(
			executeNodeRuntime(
				{ getBoard: () => Promise.resolve({ nodes: {} }) } as never,
				executeBoard as never,
				args,
			),
		).rejects.toThrow("was not found on board");
	});
});

/**
 * Remote backends resolve executeBoard with undefined metadata by design. A
 * run the tool cannot grade must never count as a pass — it recovers the
 * metadata by run id, and errors when even that fails.
 */
describe("runBoardTestsRuntime", () => {
	const args = { appId: "app-1", boardId: "board-1" };
	const board = {
		nodes: {
			a: {
				id: "a",
				name: "events_simple",
				friendly_name: "testEmptyCart",
				start: true,
			},
		},
	};
	const executeBoard = async (
		_appId: string,
		_boardId: string,
		_payload: unknown,
		_streamState: boolean,
		onId?: (id: string) => void,
	) => {
		onId?.("run-1");
		return undefined;
	};
	type ToolResult = {
		status: string;
		passed: number;
		failed: number;
		tests: Array<{
			verdict: string;
			run_id?: string;
			execution_error?: string;
		}>;
	};

	test("errors instead of passing when metadata cannot be recovered", async () => {
		const result = (await runBoardTestsRuntime(
			{
				getBoard: () => Promise.resolve(board),
				queryRun: () => Promise.resolve([]),
				listRuns: () => Promise.resolve([]),
			} as never,
			executeBoard as never,
			args,
		)) as ToolResult;

		expect(result.status).toBe("ok");
		expect(result.failed).toBe(1);
		expect(result.tests[0].verdict).toBe("error");
		expect(result.tests[0].execution_error).toContain("no metadata");
	});

	test("recovers metadata by run id and grades the run", async () => {
		const result = (await runBoardTestsRuntime(
			{
				getBoard: () => Promise.resolve(board),
				queryRun: (_meta: unknown, query: string) =>
					Promise.resolve(
						query.startsWith("message") ? [{ message: "ASSERT_OK total" }] : [],
					),
				listRuns: () => Promise.resolve([{ run_id: "run-1" }]),
			} as never,
			executeBoard as never,
			args,
		)) as ToolResult;

		expect(result.passed).toBe(1);
		expect(result.tests[0].verdict).toBe("pass");
		expect(result.tests[0].run_id).toBe("run-1");
	});
});
