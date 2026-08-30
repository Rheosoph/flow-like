import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "@flow-like/flow-like-ui/lib/schema/flow/node";
import { describe, expect, test } from "vitest";
import { enableWorkflowErrorHandling } from "../handle-errors";

function pin(
	id: string,
	name: string,
	pinType: IPinType,
	dataType: IVariableType,
	index: number,
): IPin {
	return {
		id,
		name,
		friendly_name: name,
		description: "",
		pin_type: pinType,
		data_type: dataType,
		value_type: IValueType.Normal,
		index,
		connected_to: [],
		depends_on: [],
	};
}

function board(): IBoard {
	return {
		id: "board",
		name: "Board",
		description: "",
		nodes: {
			api: {
				id: "api",
				name: "http_request",
				friendly_name: "API Call",
				category: "HTTP",
				description: "",
				coordinates: [0, 0, 0],
				layer: null,
				pins: {
					in: pin("in", "exec_in", IPinType.Input, IVariableType.Execution, 1),
					out: pin(
						"out",
						"exec_out",
						IPinType.Output,
						IVariableType.Execution,
						1,
					),
					response: pin(
						"response",
						"response",
						IPinType.Output,
						IVariableType.String,
						2,
					),
					status: pin(
						"status",
						"status",
						IPinType.Output,
						IVariableType.Integer,
						3,
					),
				},
			},
			pure: {
				id: "pure",
				name: "string_trim",
				friendly_name: "Trim",
				category: "Text",
				description: "",
				coordinates: [0, 0, 0],
				layer: null,
				pins: {
					value: pin("value", "value", IPinType.Input, IVariableType.String, 1),
				},
			},
		},
		layers: {
			fn: {
				id: "fn",
				name: "Normalize",
				type: "Function",
				coordinates: [0, 0, 0],
				pins: {},
				nodes: {},
				variables: {},
				comments: {},
			},
		},
		refs: {},
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

describe("workflow screenshot Handle Errors", () => {
	test("adds Studio's two reserved outputs with deterministic ids", () => {
		const value = board();
		const target = enableWorkflowErrorHandling(value, "API Call");
		const node = value.nodes.api;
		if (!node) throw new Error("The executable node fixture is missing.");

		expect(target).toMatchObject({ id: "api", kind: "node" });
		expect(node.pins["api-auto-handle-error"]).toEqual({
			name: "auto_handle_error",
			description: "Handles Node Errors for you.",
			pin_type: IPinType.Output,
			value_type: IValueType.Normal,
			data_type: IVariableType.Execution,
			id: "api-auto-handle-error",
			index: 4,
			connected_to: [],
			depends_on: [],
			friendly_name: "On Error",
			default_value: [102, 97, 108, 115, 101],
		});
		expect(node.pins["api-auto-handle-error-string"]).toEqual({
			name: "auto_handle_error_string",
			description: "Handles Node Errors for you.",
			pin_type: IPinType.Output,
			value_type: IValueType.Normal,
			data_type: IVariableType.String,
			id: "api-auto-handle-error-string",
			index: 5,
			connected_to: [],
			depends_on: [],
			friendly_name: "Error",
			default_value: [34, 34],
		});
	});

	test("is idempotent", () => {
		const value = board();
		enableWorkflowErrorHandling(value, "//@n:api");
		const once = JSON.stringify(value.nodes.api?.pins);

		enableWorkflowErrorHandling(value, "api");
		expect(JSON.stringify(value.nodes.api?.pins)).toBe(once);
		expect(
			Object.values(value.nodes.api?.pins ?? {}).filter((pin) =>
				pin.name.startsWith("auto_handle_error"),
			),
		).toHaveLength(2);
	});

	test("appends after sparse output indices", () => {
		const value = board();
		const node = value.nodes.api;
		if (!node) throw new Error("The executable node fixture is missing.");
		const status = node.pins.status;
		if (!status) throw new Error("The status pin fixture is missing.");
		status.index = 20;

		enableWorkflowErrorHandling(value, "API Call");
		expect(node.pins["api-auto-handle-error"]?.index).toBe(21);
		expect(node.pins["api-auto-handle-error-string"]?.index).toBe(22);
	});

	test("rejects layers and pure nodes", () => {
		expect(() => enableWorkflowErrorHandling(board(), "Normalize")).toThrow(
			"must select a workflow node",
		);
		expect(() => enableWorkflowErrorHandling(board(), "Trim")).toThrow(
			"requires a node with an Execution pin",
		);
	});
});
