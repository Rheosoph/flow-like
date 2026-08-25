import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import {
	type INode,
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "@flow-like/flow-like-ui/lib/schema/flow/node";
import { resolveWorkflowFocus } from "./focus";
import type { WorkflowFocusTarget } from "./types";

const HANDLE_ERRORS_DESCRIPTION = "Handles Node Errors for you.";

function usedPinIds(board: IBoard): Set<string> {
	const ids = new Set<string>();
	for (const node of Object.values(board.nodes)) {
		for (const pin of Object.values(node.pins)) ids.add(pin.id);
	}
	for (const layer of Object.values(board.layers)) {
		for (const pin of Object.values(layer.pins)) ids.add(pin.id);
		for (const node of Object.values(layer.nodes)) {
			for (const pin of Object.values(node.pins)) ids.add(pin.id);
		}
	}
	return ids;
}

function uniquePinId(base: string, usedIds: Set<string>): string {
	let candidate = base;
	let suffix = 2;
	while (usedIds.has(candidate)) {
		candidate = `${base}-${suffix}`;
		suffix += 1;
	}
	usedIds.add(candidate);
	return candidate;
}

function ensureHandleErrorOutput(
	node: INode,
	usedIds: Set<string>,
	definition: {
		name: string;
		idSuffix: string;
		friendlyName: string;
		dataType: IVariableType;
		defaultValue: number[];
	},
): IPin {
	const existing = Object.values(node.pins).find(
		(pin) => pin.name === definition.name && pin.pin_type === IPinType.Output,
	);
	if (existing) return existing;
	const nextOutputIndex =
		Math.max(
			-1,
			...Object.values(node.pins)
				.filter((candidate) => candidate.pin_type === IPinType.Output)
				.map((candidate) => candidate.index),
		) + 1;

	const pin: IPin = {
		name: definition.name,
		description: HANDLE_ERRORS_DESCRIPTION,
		pin_type: IPinType.Output,
		value_type: IValueType.Normal,
		data_type: definition.dataType,
		id: uniquePinId(`${node.id}-${definition.idSuffix}`, usedIds),
		index: nextOutputIndex,
		connected_to: [],
		depends_on: [],
		friendly_name: definition.friendlyName,
		default_value: [...definition.defaultValue],
	};
	node.pins[pin.id] = pin;
	return pin;
}

/** Enable the same generic error outputs as Studio's Handle Errors node toggle. */
export function enableWorkflowErrorHandling(
	board: IBoard,
	selector: string,
): WorkflowFocusTarget {
	if (!selector.trim()) throw new Error("--handle-errors cannot be empty.");
	const target = resolveWorkflowFocus(board, selector);
	if (target.kind !== "node") {
		throw new Error(
			`--handle-errors must select a workflow node; ${JSON.stringify(target.label)} is a layer.`,
		);
	}

	const node = board.nodes[target.id];
	if (!node) {
		throw new Error(
			`The node selected by --handle-errors no longer exists: ${target.id}.`,
		);
	}
	const isExecutable = Object.values(node.pins).some(
		(pin) => pin.data_type === IVariableType.Execution,
	);
	if (!isExecutable) {
		throw new Error(
			`--handle-errors requires a node with an Execution pin; ${JSON.stringify(target.label)} is a pure node.`,
		);
	}

	const usedIds = usedPinIds(board);
	ensureHandleErrorOutput(node, usedIds, {
		name: "auto_handle_error",
		idSuffix: "auto-handle-error",
		friendlyName: "On Error",
		dataType: IVariableType.Execution,
		defaultValue: [102, 97, 108, 115, 101],
	});
	ensureHandleErrorOutput(node, usedIds, {
		name: "auto_handle_error_string",
		idSuffix: "auto-handle-error-string",
		friendlyName: "Error",
		dataType: IVariableType.String,
		defaultValue: [34, 34],
	});
	return target;
}
