import { describe, expect, test } from "bun:test";
import type { IBoard } from "../../lib/schema/flow/board";
import type { IBoardState } from "../../state/backend-state/board-state";
import {
	BoardEditCoordinator,
	boardEditLockKey,
} from "../flowpilot/board-edit-guard";
import { FlowPilotBoardActivity } from "./flowpilot-board-activity";
import {
	inspectFlowPilotBoard,
	inspectFlowPilotBoardWhenIdle,
} from "./flowpilot-board-inspection";

const canonical = `eventsGeneric @["submit-entry"](payload: Struct) {
  logInfo @["shared-node"](stringConcat @["same-expression"]("prefix", "value"))
}
`;
function savedBoard(): IBoard {
	return {
		id: "board",
		name: "Intake",
		version: [1, 0, 0],
		page_ids: ["page"],
		layers: {},
		nodes: {
			"submit-entry": {
				id: "submit-entry",
				name: "events_generic",
				friendly_name: "submit_ticket",
				pins: {
					exec: {
						id: "exec",
						pin_type: "Output",
						data_type: "Execution",
						connected_to: ["sink-input"],
					},
				},
			},
			"unused-entry": {
				id: "unused-entry",
				name: "events_simple",
				friendly_name: "Unused",
				pins: {},
			},
			sink: {
				id: "sink",
				name: "log_info",
				friendly_name: "Log",
				pins: {
					input: {
						id: "sink-input",
						pin_type: "Input",
						data_type: "Execution",
						connected_to: [],
					},
				},
			},
		},
	} as unknown as IBoard;
}
function reader(board = savedBoard(), source = canonical) {
	const calls: unknown[][] = [];
	const backend = {
		getBoardSummariesAuthoritative: async (...args: unknown[]) => {
			calls.push(["inventory", ...args]);
			return [{ id: board.id }];
		},
		getBoardAuthoritative: async (...args: unknown[]) => {
			calls.push(["board", ...args]);
			return structuredClone(board);
		},
		getFlowScriptAuthoritative: async (...args: unknown[]) => {
			calls.push(["source", ...args]);
			return source;
		},
	};
	return {
		calls,
		backend: backend as unknown as Pick<
			IBoardState,
			| "getBoardSummariesAuthoritative"
			| "getBoardAuthoritative"
			| "getFlowScriptAuthoritative"
		>,
	};
}
const target = { appId: "app", boardId: "board" };

describe("board inspection while a specialist owns the board", () => {
	test("reports live draft progress immediately without queueing a saved-board read", async () => {
		const coordinator = new BoardEditCoordinator();
		const activity = new FlowPilotBoardActivity();
		const release = await coordinator.acquire(boardEditLockKey("app", "board"));
		const run = activity.begin("app", "board", "edit-1", { mode: "edit" });
		run.update({
			stage: "generating",
			draft_id: "retained",
			revision: 15,
			diagnostic_count: 0,
			workspace_status: "submitted",
		});
		const { calls, backend } = reader();
		try {
			const result = await inspectFlowPilotBoardWhenIdle(backend, target, {
				coordinator,
				activity,
				assertActive: () => {},
			});
			expect(result).toMatchObject({
				status: "in_progress",
				active_run: {
					request_id: "edit-1",
					revision: 15,
					diagnostic_count: 0,
					commit_state: "not_observed",
				},
			});
			expect(result).not.toHaveProperty("flowscript");
			expect(calls).toEqual([]);
			expect(
				coordinator.tryAcquire(boardEditLockKey("app", "board")),
			).toBeUndefined();
		} finally {
			run.finish();
			release();
		}
		expect(
			(
				await inspectFlowPilotBoardWhenIdle(backend, target, {
					coordinator,
					activity,
					assertActive: () => {},
				})
			).status,
		).toBe("ok");
		expect(calls.length).toBe(4);
	});

	test("an idle inspection reserves the board before an edit can race its readback", async () => {
		const coordinator = new BoardEditCoordinator();
		const { backend } = reader();
		let editing = false;
		const inspection = inspectFlowPilotBoardWhenIdle(backend, target, {
			coordinator,
			assertActive: () => {},
		});
		const edit = coordinator
			.acquire(boardEditLockKey("app", "board"))
			.then((release) => {
				editing = true;
				return release;
			});
		expect(editing).toBe(false);
		expect((await inspection).status).toBe("ok");
		(await edit)();
		expect(editing).toBe(true);
	});

	test("a sibling board edit does not block this board's persisted inspection", async () => {
		const coordinator = new BoardEditCoordinator();
		const release = await coordinator.acquire(
			boardEditLockKey("app", "sibling"),
		);
		try {
			const { backend } = reader();
			expect(
				(
					await inspectFlowPilotBoardWhenIdle(backend, target, {
						coordinator,
						assertActive: () => {},
					})
				).status,
			).toBe("ok");
		} finally {
			release();
		}
	});

	test("cancellation reports settling ownership until the old edit actually releases", async () => {
		const coordinator = new BoardEditCoordinator();
		const activity = new FlowPilotBoardActivity();
		const controller = new AbortController();
		const release = await coordinator.acquire(
			boardEditLockKey("app", "board"),
			{ signal: controller.signal },
		);
		const run = activity.begin("app", "board", "edit-1", {
			mode: "edit",
			signal: controller.signal,
		});
		const { backend, calls } = reader();
		controller.abort();
		try {
			expect(
				await inspectFlowPilotBoardWhenIdle(backend, target, {
					coordinator,
					activity,
					assertActive: () => {},
				}),
			).toMatchObject({
				status: "in_progress",
				active_run: { stage: "cancelling" },
			});
			expect(calls).toEqual([]);
		} finally {
			run.finish();
			release();
		}
	});

	test("untracked board operations report contention without leaking another board's draft", async () => {
		const coordinator = new BoardEditCoordinator();
		const activity = new FlowPilotBoardActivity();
		activity
			.begin("other-app", "board", "foreign", { mode: "edit" })
			.update({ draft_id: "secret" });
		const release = await coordinator.acquire(boardEditLockKey("app", "board"));
		try {
			const { backend } = reader();
			expect(
				await inspectFlowPilotBoardWhenIdle(backend, target, {
					coordinator,
					activity,
					assertActive: () => {},
				}),
			).toMatchObject({ status: "in_progress", active_run: null });
		} finally {
			release();
		}
	});

	test("cancelled inspections cannot read the board and always release their reservation", async () => {
		const coordinator = new BoardEditCoordinator();
		const { backend, calls } = reader();
		let assertions = 0;
		await expect(
			inspectFlowPilotBoardWhenIdle(backend, target, {
				coordinator,
				assertActive: () => {
					if (++assertions > 1) throw new Error("cancelled");
				},
			}),
		).rejects.toThrow("cancelled");
		expect(calls).toEqual([]);
		const reservation = coordinator.tryAcquire(
			boardEditLockKey("app", "board"),
		);
		expect(reservation).toBeDefined();
		(await reservation)?.();
	});
});

describe("direct authoritative board inspection", () => {
	test("returns exact canonical source and entry IDs without a specialist or writes", async () => {
		const board = savedBoard();
		const before = JSON.stringify(board);
		const { calls, backend } = reader(board);
		const result = await inspectFlowPilotBoard(backend, target);
		expect(calls).toEqual([
			["inventory", "app"],
			["board", "app", "board"],
			["source", "app", "board", undefined, true],
			["board", "app", "board"],
		]);
		expect(result).toMatchObject({
			status: "ok",
			mode: "inspect",
			app_id: "app",
			board_id: "board",
			read_only: true,
			board: { node_count: 3, layer_count: 0 },
			flowscript: {
				kind: "canonical_serialization",
				text: canonical,
				complete: true,
				anchors: true,
			},
			diagnostics: { status: "unavailable" },
			entry_node_count: 2,
			runnable_entry_node_count: 1,
			event_nodes: [
				{
					id: "submit-entry",
					node_type: "events_generic",
					created_this_run: false,
				},
			],
			coverage: { complete: true, truncated_fields: [] },
		});
		expect(result).not.toHaveProperty("applied");
		expect(result).not.toHaveProperty("compile_valid");
		expect(result).not.toHaveProperty("specialist_message");
		expect(JSON.stringify(board)).toBe(before);
		if (result.status !== "ok") throw new Error("Expected inspection");
		expect(result.event_nodes[0].supported_event_types).toContain("api");
		expect(
			result.entry_nodes.find((entry) => entry.id === "unused-entry")
				?.connected_execution_output,
		).toBe(false);
		expect(result.flowscript.sha256).toMatch(/^[a-f0-9]{64}$/);
		expect(result.serialization_note).toContain("machine identity");
		expect(result.serialization_note).toContain("shared graph nodes");
	});

	test("exposes one shared getter producer and an unused getter without rewriting canonical expressions", async () => {
		const board = savedBoard();
		board.nodes.getter = {
			id: "shared-getter",
			name: "a2ui_get_element_value",
			pins: {
				value: {
					id: "shared-value",
					pin_type: "Output",
					data_type: "Struct",
					connected_to: ["consumer-a", "consumer-b"],
				},
			},
		} as never;
		board.nodes.unused = {
			id: "unused-getter",
			name: "a2ui_get_element_value",
			pins: { value: { id: "unused-value", connected_to: [] } },
		} as never;
		const source =
			'use ui.*\neventsGeneric @["submit-entry"]() { consume(ui.getElementValue @["shared-getter"]()); consume(ui.getElementValue @["shared-getter"]()); if (true) {} else {} }';
		const { backend } = reader(board, source);
		const result = await inspectFlowPilotBoard(backend, target);
		if (result.status !== "ok") throw new Error("Expected inspection");
		expect(result.flowscript.text).toBe(source);
		// The fixture pins deliberately omit required IPin fields, so their facts carry the gaps.
		expect(
			result.node_facts.filter(
				(node) => node.node_type === "a2ui_get_element_value",
			),
		).toEqual<unknown>([
			{
				id: "shared-getter",
				node_type: "a2ui_get_element_value",
				layer_id: null,
				pins: [
					{
						id: "shared-value",
						name: undefined,
						pin_type: "Output",
						data_type: "Struct",
						connected_to: ["consumer-a", "consumer-b"],
						depends_on: undefined,
					},
				],
			},
			{
				id: "unused-getter",
				node_type: "a2ui_get_element_value",
				layer_id: null,
				pins: [
					{
						id: "unused-value",
						name: undefined,
						pin_type: undefined,
						data_type: undefined,
						connected_to: [],
						depends_on: undefined,
					},
				],
			},
		]);
		expect(result.serialization_note).toContain(
			"Wildcard imports, empty rendered branches",
		);
	});

	test("does not read an ambient board without both exact IDs", async () => {
		const { backend, calls } = reader();
		for (const scope of [
			{ appId: "", boardId: "board" },
			{ appId: "app", boardId: "" },
		]) {
			expect(await inspectFlowPilotBoard(backend, scope)).toMatchObject({
				status: "error",
				code: "FLOWPILOT_BOARD_INSPECT_TARGET_REQUIRED",
			});
		}
		expect(calls).toHaveLength(0);
	});

	test("checks app ownership before a native id-only board read could expose another app", async () => {
		const { backend, calls } = reader();
		backend.getBoardSummariesAuthoritative = async () => [];
		expect(await inspectFlowPilotBoard(backend, target)).toMatchObject({
			status: "error",
			code: "FLOWPILOT_BOARD_INSPECT_TARGET_MISMATCH",
		});
		expect(calls).toHaveLength(0);
	});

	test("rejects mismatched identities and authoritative read failures", async () => {
		const { backend, calls } = reader();
		backend.getBoardAuthoritative = async () => ({
			...savedBoard(),
			id: "other",
		});
		expect(await inspectFlowPilotBoard(backend, target)).toMatchObject({
			status: "error",
			code: "FLOWPILOT_BOARD_INSPECT_TARGET_MISMATCH",
		});
		expect(calls).toEqual([["inventory", "app"]]);
		for (const method of [
			"getBoardSummariesAuthoritative",
			"getBoardAuthoritative",
			"getFlowScriptAuthoritative",
		] as const) {
			const { backend: failing } = reader();
			failing[method] = async () => {
				throw new Error("private backend details");
			};
			const result = await inspectFlowPilotBoard(failing, target);
			expect(result).toMatchObject({
				status: "error",
				code: "FLOWPILOT_BOARD_INSPECT_READ_FAILED",
			});
			expect(JSON.stringify(result)).not.toContain("private backend details");
		}
	});

	test("rejects a board/source snapshot that changes during the read", async () => {
		const { backend } = reader();
		let reads = 0;
		backend.getBoardAuthoritative = async () => ({
			...savedBoard(),
			name: ++reads === 1 ? "before" : "after",
		});
		const result = await inspectFlowPilotBoard(backend, target);
		expect(result).toMatchObject({
			status: "error",
			code: "FLOWPILOT_BOARD_INSPECT_SNAPSHOT_CHANGED",
		});
		expect(result).not.toHaveProperty("flowscript");
		expect(result).not.toHaveProperty("event_nodes");
	});

	test("preserves exact source prefixes and whole IDs while reporting bounded coverage", async () => {
		const board = savedBoard();
		board.layers.shared = { nodes: { sink: board.nodes.sink } } as never;
		for (let index = 0; index < 90; index++) {
			const id = `entry-${index}`;
			board.nodes[id] = {
				...board.nodes["unused-entry"],
				id,
				friendly_name: "n".repeat(900),
			};
		}
		const source = canonical + "😀\n".repeat(30_000);
		const { backend } = reader(board, source);
		const result = await inspectFlowPilotBoard(backend, target);
		if (result.status !== "ok") throw new Error("Expected inspection");
		expect(result.board.node_count).toBe(93);
		expect(result.flowscript.complete).toBe(false);
		expect(source.startsWith(result.flowscript.text)).toBe(true);
		expect(result.coverage.complete).toBe(false);
		expect(result.coverage.truncated_fields).toContain("entry_nodes");
		expect(result.entry_nodes.length).toBeLessThan(64);
		for (const entry of result.entry_nodes)
			expect(board.nodes[entry.id]).toBeDefined();
		expect(
			new TextEncoder().encode(JSON.stringify(result)).length,
		).toBeLessThan(65_000);
	});
});
