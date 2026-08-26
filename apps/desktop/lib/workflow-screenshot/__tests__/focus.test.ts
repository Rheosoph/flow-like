import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import { describe, expect, test } from "vitest";
import {
	defaultWorkflowFocus,
	describeWorkflowNodes,
	resolveWorkflowFocus,
	workflowFocusSentinelId,
} from "../focus";

function board(): IBoard {
	return {
		id: "board",
		name: "Board",
		description: "",
		nodes: {
			"node-a": {
				id: "node-a",
				name: "string_trim",
				friendly_name: "Trim report",
				category: "Text",
				description: "",
				coordinates: [0, 0, 0],
				pins: {},
				layer: null,
			},
			"node-b": {
				id: "node-b",
				name: "log",
				friendly_name: "Write log",
				category: "Debug",
				description: "",
				coordinates: [0, 0, 0],
				pins: {},
				layer: "layer-a",
			},
		},
		layers: {
			"layer-a": {
				id: "layer-a",
				name: "Normalize",
				type: "Function",
				coordinates: [0, 0, 0],
				pins: {},
				nodes: {},
				variables: {},
				comments: {},
			},
		},
		refs: { "15666264297751397251": '{"type":"string"}' },
		comments: {},
		variables: {},
		viewport: [0, 0, 1],
		version: [0, 0, 1],
		stage: "Dev",
		log_level: "Info",
		execution_mode: "Local",
		page_ids: [],
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	} as unknown as IBoard;
}

describe("workflow focus resolution", () => {
	test("resolves anchors, friendly names, and layers", () => {
		expect(resolveWorkflowFocus(board(), "//@n:node-a")).toMatchObject({
			id: "node-a",
			matchedBy: "anchor",
		});
		expect(resolveWorkflowFocus(board(), "write LOG").id).toBe("node-b");
		expect(resolveWorkflowFocus(board(), "Normalize")).toMatchObject({
			id: "layer-a",
			kind: "layer",
		});
		expect(resolveWorkflowFocus(board(), "//@l:layer-a")).toMatchObject({
			id: "layer-a",
			kind: "layer",
			matchedBy: "anchor",
		});
	});

	test("does not mistake the board's content-addressed refs for source aliases", () => {
		expect(describeWorkflowNodes(board())).toContainEqual(
			expect.objectContaining({ id: "node-a" }),
		);
		expect(() => resolveWorkflowFocus(board(), "15666264297751397251")).toThrow(
			"No workflow node or layer matches",
		);
	});

	test("uses a function boundary as the focus-ready sentinel", () => {
		const value = board();
		const target = resolveWorkflowFocus(value, "Normalize");
		expect(workflowFocusSentinelId(value, target)).toBe("layer-a-input");
	});

	test("opens a deterministic function when the root canvas is empty", () => {
		const value = board();
		expect(defaultWorkflowFocus(value)).toBeUndefined();
		const nestedNode = value.nodes["node-b"];
		if (!nestedNode) throw new Error("The nested node fixture is missing.");
		value.nodes = { "node-b": nestedNode };
		expect(defaultWorkflowFocus(value)).toMatchObject({
			id: "layer-a",
			kind: "layer",
			matchedBy: "default",
		});
	});

	test("refuses ambiguous names", () => {
		const value = board();
		const logNode = value.nodes["node-b"];
		if (!logNode) throw new Error("The log node fixture is missing.");
		value.nodes["node-c"] = {
			...logNode,
			id: "node-c",
		};
		expect(() => resolveWorkflowFocus(value, "log")).toThrow("is ambiguous");
	});
});
