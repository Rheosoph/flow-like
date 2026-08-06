import {
	addNodeCommand,
	connectPinsCommand,
	disconnectPinsCommand,
	upsertLayerCommand,
} from "./command/generic-command";
import { type IBoard, type ILayer, ILayerType } from "./schema/flow/board";
import type { IGenericCommand } from "./schema/flow/board/commands/generic-command";
import type { INode } from "./schema/flow/node";
import { type IPin, IPinType, IVariableType } from "./schema/flow/pin";
import { convertJsonToUint8Array } from "./uint8";

export const CALL_FUNCTION_NODE_NAME = "control_call_function";
const FUNCTION_LAYER_PIN = "function_layer_id";

export interface LayerToFunctionPlan {
	commands: IGenericCommand[];
	callNode: INode;
	layer: ILayer;
	/** Boundary pins that had to be renamed because a function signature needs unique pin names. */
	renamedPins: number;
	/** External connections that were moved from the layer boundary onto the call node. */
	movedConnections: number;
}

export type LayerToFunctionError =
	| { reason: "already_function" }
	| { reason: "missing_catalog_node" }
	| { reason: "multiple_exec_inputs"; count: number }
	| { reason: "missing_exec_input" };

export type LayerToFunctionResult =
	| { ok: true; plan: LayerToFunctionPlan }
	| { ok: false; error: LayerToFunctionError };

export function layerToFunctionErrorMessage(
	error: LayerToFunctionError,
): string {
	switch (error.reason) {
		case "already_function":
			return "This layer already is a function";
		case "missing_catalog_node":
			return "The Call Function node is unavailable in this app";
		case "multiple_exec_inputs":
			return `A function has a single entry point, this layer has ${error.count} execution inputs. Merge them before converting.`;
		case "missing_exec_input":
			return "A function is entered through an execution input, this layer has none. Connect one before converting.";
	}
}

interface PinOwner {
	id: string;
	layerId?: string;
}

function collectPinOwners(board: IBoard): Map<string, PinOwner> {
	const owners = new Map<string, PinOwner>();

	for (const node of Object.values(board.nodes ?? {})) {
		for (const pin of Object.values(node.pins ?? {})) {
			owners.set(pin.id, { id: node.id, layerId: node.layer ?? undefined });
		}
	}

	// A layer boundary pin belongs to the layer itself — its "outside" is the parent layer.
	for (const layer of Object.values(board.layers ?? {})) {
		for (const pin of Object.values(layer.pins ?? {})) {
			owners.set(pin.id, { id: layer.id, layerId: layer.id });
		}
	}

	return owners;
}

function isWithinLayer(
	board: IBoard,
	owner: PinOwner | undefined,
	layerId: string,
): boolean {
	let current = owner?.layerId;
	let guard = 0;
	while (current && guard < 64) {
		if (current === layerId) return true;
		current = board.layers?.[current]?.parent_id ?? undefined;
		guard += 1;
	}
	return false;
}

function sortedBoundaryPins(layer: ILayer, pinType: IPinType): IPin[] {
	return Object.values(layer.pins ?? {})
		.filter((pin) => pin.pin_type === pinType)
		.toSorted((a, b) => a.index - b.index || a.id.localeCompare(b.id));
}

/**
 * Give every boundary pin a unique name per direction. `control_call_function`
 * mirrors the signature by name, so colliding names would collapse two
 * parameters into one call pin.
 */
function uniqueBoundaryNames(pins: IPin[], reserved: Set<string>) {
	const renamed: IPin[] = [];
	let renames = 0;

	for (const pin of pins) {
		if (!reserved.has(pin.name)) {
			reserved.add(pin.name);
			renamed.push(pin);
			continue;
		}

		let suffix = 2;
		while (reserved.has(`${pin.name}_${suffix}`)) suffix += 1;
		reserved.add(`${pin.name}_${suffix}`);
		renames += 1;
		renamed.push({
			...pin,
			name: `${pin.name}_${suffix}`,
			friendly_name: `${pin.friendly_name} ${suffix}`,
		});
	}

	return { pins: renamed, renames };
}

function mirrorPin(pin: IPin, index: number): IPin {
	return {
		id: pin.id,
		name: pin.name,
		friendly_name: pin.friendly_name,
		description: pin.description,
		pin_type: pin.pin_type,
		data_type: pin.data_type,
		value_type: pin.value_type,
		schema: pin.schema ?? null,
		options: pin.options ?? null,
		default_value: null,
		connected_to: [],
		depends_on: [],
		index,
	};
}

/**
 * Turn a collapsed layer into a callable function.
 *
 * The layer keeps its nodes and its boundary pins — those become the function
 * signature. Because function layers are not drawn on the board, a
 * `control_call_function` node takes the layer's place and inherits every
 * connection that crossed the layer boundary, so the surrounding flow keeps
 * working.
 */
export function planLayerToFunction({
	board,
	layer,
	callFunctionTemplate,
}: {
	board: IBoard;
	layer: ILayer;
	callFunctionTemplate: INode | undefined;
}): LayerToFunctionResult {
	if (layer.type === ILayerType.Function) {
		return { ok: false, error: { reason: "already_function" } };
	}

	if (!callFunctionTemplate) {
		return { ok: false, error: { reason: "missing_catalog_node" } };
	}

	// A function is entered through exactly one execution pin: the call node
	// follows that pin to the body's entry node. Without it the body can only be
	// evaluated as a pure expression, so execution outputs would never fire.
	const execPins = Object.values(layer.pins ?? {}).filter(
		(pin) => pin.data_type === IVariableType.Execution,
	);
	const execInputs = execPins.filter((pin) => pin.pin_type === IPinType.Input);
	if (execInputs.length > 1) {
		return {
			ok: false,
			error: { reason: "multiple_exec_inputs", count: execInputs.length },
		};
	}
	if (execInputs.length === 0 && execPins.length > 0) {
		return { ok: false, error: { reason: "missing_exec_input" } };
	}

	const reserved = new Set<string>([FUNCTION_LAYER_PIN]);
	const { pins: inputs, renames: inputRenames } = uniqueBoundaryNames(
		sortedBoundaryPins(layer, IPinType.Input),
		reserved,
	);
	const { pins: outputs, renames: outputRenames } = uniqueBoundaryNames(
		sortedBoundaryPins(layer, IPinType.Output),
		new Set<string>(),
	);

	const signature = [
		// Indices mirror `CallFunctionNode::on_update` so the placed node renders
		// its pins in the same order the backend will settle on.
		...inputs.map((pin, index) => mirrorPin(pin, index + 2)),
		...outputs.map((pin, index) => mirrorPin(pin, index + 1)),
	];

	const templatePin = Object.values(callFunctionTemplate.pins ?? {}).find(
		(pin) => pin.name === FUNCTION_LAYER_PIN,
	);
	if (!templatePin) {
		return { ok: false, error: { reason: "missing_catalog_node" } };
	}

	const callPins: Record<string, IPin> = {};
	for (const pin of [
		{
			...templatePin,
			connected_to: [],
			depends_on: [],
			default_value: convertJsonToUint8Array(layer.id),
		},
		...signature,
	]) {
		callPins[pin.id] = pin;
	}

	const { command: addCall, node: callNode } = addNodeCommand({
		node: {
			...callFunctionTemplate,
			friendly_name: `Call ${layer.name}`,
			description: `Calls the function '${layer.name}'`,
			coordinates: [
				layer.coordinates?.[0] ?? 0,
				layer.coordinates?.[1] ?? 0,
				layer.coordinates?.[2] ?? 0,
			],
			comment: null,
			error: null,
			pins: callPins,
		},
		current_layer: layer.parent_id ?? undefined,
	});

	const callPinByName = new Map<string, IPin>();
	for (const pin of Object.values(callNode.pins)) {
		callPinByName.set(`${pin.pin_type}:${pin.name}`, pin);
	}

	const functionLayer: ILayer = {
		...layer,
		type: ILayerType.Function,
		parent_id: null,
		pins: Object.fromEntries(
			[...inputs, ...outputs].map((pin) => [pin.id, pin]),
		),
	};

	const owners = collectPinOwners(board);
	const disconnects: IGenericCommand[] = [];
	const connects: IGenericCommand[] = [];

	for (const pin of [...inputs, ...outputs]) {
		const mirrored = callPinByName.get(`${pin.pin_type}:${pin.name}`);
		if (!mirrored) continue;

		for (const sourceId of pin.depends_on) {
			const owner = owners.get(sourceId);
			if (!owner || isWithinLayer(board, owner, layer.id)) continue;

			disconnects.push(
				disconnectPinsCommand({
					from_node: owner.id,
					from_pin: sourceId,
					to_node: layer.id,
					to_pin: pin.id,
				}),
			);
			connects.push(
				connectPinsCommand({
					from_node: owner.id,
					from_pin: sourceId,
					to_node: callNode.id,
					to_pin: mirrored.id,
				}),
			);
		}

		for (const targetId of pin.connected_to) {
			const owner = owners.get(targetId);
			if (!owner || isWithinLayer(board, owner, layer.id)) continue;

			disconnects.push(
				disconnectPinsCommand({
					from_node: layer.id,
					from_pin: pin.id,
					to_node: owner.id,
					to_pin: targetId,
				}),
			);
			connects.push(
				connectPinsCommand({
					from_node: callNode.id,
					from_pin: mirrored.id,
					to_node: owner.id,
					to_pin: targetId,
				}),
			);
		}
	}

	return {
		ok: true,
		plan: {
			// The layer snapshot still carries the outside connections, so it has to
			// be written before they are moved over to the call node.
			commands: [
				addCall,
				upsertLayerCommand({ layer: functionLayer, node_ids: [] }),
				...disconnects,
				...connects,
			],
			callNode,
			layer: functionLayer,
			renamedPins: inputRenames + outputRenames,
			movedConnections: connects.length,
		},
	};
}
