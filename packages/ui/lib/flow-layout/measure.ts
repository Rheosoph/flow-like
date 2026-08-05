import type { ILayer } from "../schema/flow/board";
import type { INode, IPin } from "../schema/flow/node";
import { IPinType, IVariableType } from "../schema/flow/node";

/**
 * Geometry constants mirrored from the renderer. Every value here has a single
 * source of truth in a component file; changing one without the other silently
 * desynchronises layout from what the user sees.
 *
 *  - NODE_WIDTH          `.react-flow__node-default { width: 150px }`
 *                        (@xyflow/react/dist/style.css, not overridden in global.css)
 *  - PIN_ROW_HEIGHT      `top: (pin.index - 1) * 15`            (flow-pin.tsx)
 *  - NODE_CHROME_HEIGHT  `calc(rows * 15px + 1.25rem + 0.5rem)` (flow-node.tsx, layer-node.tsx)
 *  - PIN_MARGIN_TOP      `marginTop: "1.75rem"`                 (flow-pin.tsx)
 *  - HANDLE_SIZE         `width: 12` on the handle              (flow-node.tsx)
 *  - REROUTE_*           `w-4 max-w-4 max-h-3!`                 (flow-node.tsx)
 *  - ENTITY_MIN_HEIGHT   `minHeight: calc(15px + 1.25rem + 0.5rem)` (layer-node.tsx)
 */
export const NODE_WIDTH = 150;
export const PIN_ROW_HEIGHT = 15;
export const NODE_CHROME_HEIGHT = 28;
export const PIN_MARGIN_TOP = 28;
export const HANDLE_SIZE = 12;
export const REROUTE_WIDTH = 16;
export const REROUTE_HEIGHT = 12;
export const ENTITY_MIN_HEIGHT = PIN_ROW_HEIGHT + NODE_CHROME_HEIGHT;

export const REROUTE_NODE_NAME = "reroute";
const CALL_FUNCTION_NODE_NAME = "control_call_function";
const CALL_FUNCTION_HIDDEN_PIN = "function_layer_id";

export function isRerouteNode(node: Pick<INode, "name">): boolean {
	return node.name === REROUTE_NODE_NAME;
}

export function isExecPin(pin: IPin): boolean {
	return pin.data_type === IVariableType.Execution;
}

export function isInputPin(pin: IPin): boolean {
	return pin.pin_type === IPinType.Input;
}

export function isOutputPin(pin: IPin): boolean {
	return pin.pin_type === IPinType.Output;
}

function comparePins(a: IPin, b: IPin): number {
	if (a.pin_type === IPinType.Input && b.pin_type === IPinType.Output)
		return -1;
	if (a.pin_type === IPinType.Output && b.pin_type === IPinType.Input) return 1;
	return a.index - b.index;
}

/**
 * Mirrors `visiblePins` in flow-node.tsx: `control_call_function` hides its
 * `function_layer_id` pin and re-indexes the remaining inputs from 1.
 */
export function visiblePinsOf(node: INode): IPin[] {
	const all = Object.values(node.pins ?? {});
	if (node.name !== CALL_FUNCTION_NODE_NAME) return all;

	let inputIndex = 0;
	return all
		.filter((pin) => pin.name !== CALL_FUNCTION_HIDDEN_PIN)
		.sort(comparePins)
		.map((pin) => {
			if (pin.pin_type !== IPinType.Input) return pin;
			inputIndex += 1;
			return { ...pin, index: inputIndex };
		});
}

/**
 * Mirrors `parsePins` in flow-node.tsx. Consecutive pins sharing
 * `${name}_${pin_type}` render an extra "add pin" action row, so the rendered
 * row count is not simply the pin count.
 */
export function countRenderedPinRows(node: INode): number {
	const sorted = [...visiblePinsOf(node)].sort(comparePins);

	let inputRows = 0;
	let outputRows = 0;
	let runKey = "";
	let runLength = 0;
	let runIsInput = false;

	const flushRun = () => {
		if (runLength < 2) return;
		if (runIsInput) inputRows += 1;
		else outputRows += 1;
	};

	for (const pin of sorted) {
		const key = `${pin.name}_${pin.pin_type}`;
		if (key !== runKey) {
			flushRun();
			runKey = key;
			runLength = 0;
			runIsInput = isInputPin(pin);
		}
		runLength += 1;
		if (isInputPin(pin)) inputRows += 1;
		else outputRows += 1;
	}
	flushRun();

	return Math.max(inputRows, outputRows);
}

export interface NodeBox {
	width: number;
	height: number;
}

export function measureNodeBox(node: INode, isEntity = false): NodeBox {
	if (!isEntity && isRerouteNode(node)) {
		return { width: REROUTE_WIDTH, height: REROUTE_HEIGHT };
	}

	const rows = countRenderedPinRows(node);
	const height = rows * PIN_ROW_HEIGHT + NODE_CHROME_HEIGHT;

	return {
		width: NODE_WIDTH,
		height: isEntity ? Math.max(ENTITY_MIN_HEIGHT, height) : height,
	};
}

export function measureLayerBox(layer: ILayer): NodeBox {
	const pins = Object.values(layer.pins ?? {});
	const rows = Math.max(
		pins.filter(isInputPin).length,
		pins.filter(isOutputPin).length,
	);
	return {
		width: NODE_WIDTH,
		height: Math.max(
			ENTITY_MIN_HEIGHT,
			rows * PIN_ROW_HEIGHT + NODE_CHROME_HEIGHT,
		),
	};
}

/**
 * Distance from the node's top edge to a pin handle's visual centre.
 *
 * The handle is centred on that point by xyflow's own
 * `.react-flow__handle-left/right { transform: translate(-50%, -50%) }`, which
 * the inline style in flow-pin.tsx does not override — so do NOT add half the
 * handle size here. Reroute handles are unoffset and sit at the node's midpoint.
 */
export function pinOffsetY(pin: IPin, isReroute = false): number {
	if (isReroute) return REROUTE_HEIGHT / 2;
	return PIN_MARGIN_TOP + (pin.index - 1) * PIN_ROW_HEIGHT;
}

/** Guards against reading a node that react-flow has not finished mounting. */
export function isPlausibleSize(
	width: number | null | undefined,
	height: number | null | undefined,
): boolean {
	return (
		typeof width === "number" &&
		typeof height === "number" &&
		Number.isFinite(width) &&
		Number.isFinite(height) &&
		width >= 8 &&
		height >= 8
	);
}
