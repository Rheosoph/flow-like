import { createId } from "@paralleldrive/cuid2";
import type { Connection, Edge } from "@xyflow/react";
import {
	addNodeCommand,
	connectPinsCommand,
	disconnectPinsCommand,
	removeCommentCommand,
	removeLayerCommand,
	removeNodeCommand,
	updateNodeCommand,
	upsertLayerCommand,
} from ".";
import { buildConnectionCommands } from "./board-canvas/connection-commands";
import {
	doPinsMatch,
	isOpenObjectSchema,
	pendingEdge,
} from "./flow-board-utils";
import {
	ICommandType,
	type IGenericCommand,
} from "./schema/flow/board/commands/generic-command";
import { ILayerType } from "./schema/flow/board/commands/upsert-layer";
import type { INode } from "./schema/flow/node";
import type { IPin } from "./schema/flow/pin";
import { IPinType, IValueType, IVariableType } from "./schema/flow/pin";
import type { ILayer } from "./schema/flow/run";

interface PlaceNodeParams {
	node: INode;
	position: { x: number; y: number };
	droppedPin: IPin | undefined;
	currentLayer: string | undefined;
	refs: Record<string, any>;
	boardNodes: Record<string, INode>;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
	executeCommands: (commands: IGenericCommand[]) => Promise<unknown>;
}

export async function handlePlaceNode({
	node,
	position,
	droppedPin,
	currentLayer,
	refs,
	boardNodes,
	pinCache,
	executeCommands,
}: PlaceNodeParams) {
	const template = structuredClone(node);
	for (const pin of Object.values(template.pins)) {
		pin.connected_to = [];
		pin.depends_on = [];
	}
	const result = addNodeCommand({
		node: { ...template, coordinates: [position.x, position.y, 0] },
		current_layer: currentLayer,
	});
	const commands = [result.command];
	if (droppedPin) {
		const added = result.node;
		const nextCache = new Map(pinCache);
		for (const pin of Object.values(added.pins))
			nextCache.set(pin.id, [pin, added, true]);
		const refOut = droppedPin.id.startsWith("ref_out_");
		const refIn = droppedPin.id.startsWith("ref_in_");
		const owner =
			refOut || refIn
				? boardNodes[droppedPin.id.slice(refOut ? 8 : 7)]
				: pinCache.get(droppedPin.id)?.[1];
		const target =
			refOut || refIn
				? undefined
				: findDropTargetPin(added.pins, droppedPin, refs);
		if (!owner || (!(refOut || refIn) && !target))
			throw new Error("The dropped pin is no longer available or compatible.");
		const output =
			refOut || (!refIn && droppedPin.pin_type === IPinType.Output);
		const addedHandle = refOut
			? `ref_in_${added.id}`
			: refIn
				? `ref_out_${added.id}`
				: (target?.id ?? "");
		const connection = {
			source: output ? owner.id : added.id,
			sourceHandle: output ? droppedPin.id : addedHandle,
			target: output ? added.id : owner.id,
			targetHandle: output ? addedHandle : droppedPin.id,
		};
		const connected = buildConnectionCommands({
			connection,
			boardNodes: { ...boardNodes, [added.id]: added },
			pinCache: nextCache,
			refs,
		});
		if (!connected)
			throw new Error("The dropped pin is no longer available or compatible.");
		commands.push(...connected);
	}
	await executeCommands(commands);
}

/** Reference pins a preset node arrives bound through (Get/Set Variable, Call Function/Event). */
const BINDING_PINS = new Set(["var_ref", "fn_ref", "function_layer_id"]);

const isBoundReference = (pin: IPin) =>
	BINDING_PINS.has(pin.name) && (pin.default_value?.length ?? 0) > 0;

/**
 * The pin on `pins` that a wire dropped from `droppedPin` connects to when the node is placed.
 * Only pins the editor's own connection check (`doPinsMatch`) accepts qualify, so a drop never
 * wires what a manual connect would reject, and a preset node's bound reference is never rewired.
 */
export function findDropTargetPin(
	pins: Record<string, IPin>,
	droppedPin: IPin,
	refs: Record<string, string>,
): IPin | undefined {
	return matchingPins(pins, {
		pinType: droppedPin.pin_type === "Input" ? "Output" : "Input",
		pinValueType: droppedPin.value_type,
		pinDataType: droppedPin.data_type,
		schema: refs?.[droppedPin.schema ?? ""] ?? droppedPin.schema,
		options: droppedPin.options,
		refs,
	}).find(
		(pin) => !isBoundReference(pin) && doPinsMatch(pin, droppedPin, refs),
	);
}

interface FindMatchingPinParams {
	pinType: string;
	pinValueType: string;
	pinDataType: string;
	schema: any;
	options: any;
	refs: Record<string, any>;
}

function matchingPins(
	pins: Record<string, IPin>,
	{
		pinType,
		pinValueType,
		pinDataType,
		schema,
		options,
		refs,
	}: FindMatchingPinParams,
): IPin[] {
	// An open-object schema declares that the shape is open, not a contract to match, so it counts
	// as "no schema" here exactly as it does in `doPinsMatch`. Without this, dragging off a typed
	// struct pin would stop offering Break Struct, whose `struct_in` accepts any struct.
	const droppedSchema =
		typeof schema === "string" && !isOpenObjectSchema(schema)
			? schema
			: undefined;

	// `pins` is keyed by cuid, so its iteration order is arbitrary. Pin index is the order the
	// node declares — and the order the user sees — so a node with two pins a drop could land on
	// (Cast to Struct's `struct_in` and `struct_shape`) connects to the first one every time
	// instead of whichever the map happened to yield.
	const ordered = Object.values(pins).sort((a, b) => a.index - b.index);

	return ordered.filter((pin) => {
		const rawPinSchema = refs?.[pin.schema ?? ""] ?? pin.schema;
		const pinSchema =
			typeof rawPinSchema === "string" && !isOpenObjectSchema(rawPinSchema)
				? rawPinSchema
				: undefined;

		// Only two concrete schemas can contradict each other — the same rule `doPinsMatch` and
		// `schemas_are_compatible` apply when the connection is actually made.
		if (
			droppedSchema &&
			pinSchema &&
			droppedSchema !== pinSchema &&
			(pin.options?.enforce_schema || options?.enforce_schema) &&
			pin.data_type !== IVariableType.Generic &&
			pinDataType !== IVariableType.Generic
		)
			return false;
		if (pin.pin_type !== pinType) return false;
		if (pin.value_type !== pinValueType) {
			if (
				pinDataType !== IVariableType.Generic &&
				pin.data_type !== IVariableType.Generic
			)
				return false;
			const sourceEnforces = options?.enforce_generic_value_type ?? false;
			const targetEnforces = pin.options?.enforce_generic_value_type ?? false;
			if (sourceEnforces || targetEnforces) {
				if (sourceEnforces && targetEnforces) return false;
				if (sourceEnforces && pin.data_type !== IVariableType.Generic)
					return false;
				if (targetEnforces && pinDataType !== IVariableType.Generic)
					return false;
			}
		}
		if (
			pin.data_type === IVariableType.Generic &&
			pinDataType !== IVariableType.Execution
		)
			return true;
		if (
			pinDataType === IVariableType.Generic &&
			pin.data_type !== IVariableType.Execution
		)
			return true;
		return pin.data_type === pinDataType;
	});
}

interface PlacePlaceholderParams {
	name: string;
	position: { x: number; y: number };
	droppedPin: IPin | undefined;
	currentLayer: string | undefined;
	refs: Record<string, any>;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
	delayNode: INode | undefined;
	executeCommand: (command: any, refetch?: boolean) => Promise<any>;
	executeCommands: (commands: any[]) => Promise<any>;
}

export async function handlePlacePlaceholder({
	name,
	position,
	droppedPin,
	currentLayer,
	refs,
	pinCache,
	delayNode,
	executeCommand,
	executeCommands,
}: PlacePlaceholderParams) {
	const layerId = createId();

	const execInPin: IPin = {
		id: createId(),
		name: "exec_in",
		friendly_name: "Exec In",
		connected_to: [],
		depends_on: [],
		description: "",
		index: 1,
		pin_type: IPinType.Input,
		value_type: IValueType.Normal,
		data_type: IVariableType.Execution,
		default_value: null,
	};

	const execOutPin: IPin = {
		...execInPin,
		id: createId(),
		pin_type: IPinType.Output,
		name: "exec_out",
		friendly_name: "Exec Out",
		index: 1,
	};

	let dataPin: IPin | undefined;
	let connectToPinId: string | undefined;

	if (droppedPin) {
		const oppositeType =
			droppedPin.pin_type === "Input" ? IPinType.Output : IPinType.Input;

		if (droppedPin.data_type === IVariableType.Execution) {
			connectToPinId =
				oppositeType === IPinType.Input ? execInPin.id : execOutPin.id;
		} else {
			const resolvedSchema =
				typeof droppedPin.schema === "string"
					? (refs?.[droppedPin.schema] ?? droppedPin.schema)
					: droppedPin.schema;

			dataPin = {
				id: createId(),
				name: oppositeType === IPinType.Input ? "in" : "out",
				friendly_name: oppositeType === IPinType.Input ? "In" : "Out",
				connected_to: [],
				depends_on: [],
				description: "",
				index: 2,
				pin_type: oppositeType,
				value_type: droppedPin.value_type,
				data_type: droppedPin.data_type,
				default_value: null,
				...(resolvedSchema ? { schema: resolvedSchema } : {}),
				...(droppedPin.options ? { options: droppedPin.options } : {}),
			};

			connectToPinId = dataPin.id;
		}
	}

	const pins: Record<string, IPin> = {
		[execInPin.id]: execInPin,
		[execOutPin.id]: execOutPin,
		...(dataPin ? { [dataPin.id]: dataPin } : {}),
	};

	const newLayerCommand = upsertLayerCommand({
		current_layer: currentLayer,
		layer: {
			comments: {},
			coordinates: [position.x, position.y, 0],
			id: layerId,
			name,
			nodes: {},
			pins,
			type: ILayerType.Collapsed,
			variables: {},
			parent_id: currentLayer,
		},
		node_ids: [],
	});

	const newLayerResult = await executeCommand(newLayerCommand, false);
	const newLayer: ILayer = newLayerResult.layer;

	if (delayNode) {
		await connectDelayNodeToLayer(
			delayNode,
			newLayer,
			execInPin,
			execOutPin,
			executeCommand,
			executeCommands,
		);
	}

	if (!droppedPin) return;

	await connectLayerToDroppedPin(
		newLayer,
		droppedPin,
		pinCache,
		executeCommand,
	);
}

async function connectDelayNodeToLayer(
	delayNode: INode,
	newLayer: ILayer,
	execInPin: IPin,
	execOutPin: IPin,
	executeCommand: (command: any) => Promise<any>,
	executeCommands: (commands: any[]) => Promise<any>,
) {
	const placeDelayCommand = addNodeCommand({
		node: delayNode,
		current_layer: newLayer.id,
	});

	const placedNode = await executeCommand(placeDelayCommand.command);
	const newNode: INode = placedNode.node;

	const newNodeInPin = Object.values(newNode.pins).find(
		(pin) =>
			pin.pin_type === IPinType.Input &&
			pin.data_type === IVariableType.Execution,
	);
	const newNodeOutPin = Object.values(newNode.pins).find(
		(pin) =>
			pin.pin_type === IPinType.Output &&
			pin.data_type === IVariableType.Execution,
	);

	const connectOutput = connectPinsCommand({
		from_node: newNode.id,
		from_pin: newNodeOutPin!.id,
		to_node: newLayer.id,
		to_pin: execOutPin.id,
	});

	const connectInput = connectPinsCommand({
		to_node: newNode.id,
		to_pin: newNodeInPin!.id,
		from_node: newLayer.id,
		from_pin: execInPin.id,
	});

	await executeCommands([connectOutput, connectInput]);
}

async function connectLayerToDroppedPin(
	newLayer: ILayer,
	droppedPin: IPin,
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>,
	executeCommand: (command: any) => Promise<any>,
) {
	const pinType = droppedPin.pin_type === "Input" ? "Output" : "Input";
	const pinValueType = droppedPin.value_type;
	const pinDataType = droppedPin.data_type;
	const options = droppedPin.options;

	const pin = Object.values(newLayer.pins).find((pin) => {
		if (pin.pin_type !== pinType) return false;
		if (pin.value_type !== pinValueType) {
			if (
				pinDataType !== IVariableType.Generic &&
				pin.data_type !== IVariableType.Generic
			)
				return false;
			const sourceEnforces = options?.enforce_generic_value_type ?? false;
			const targetEnforces = pin.options?.enforce_generic_value_type ?? false;
			if (sourceEnforces || targetEnforces) {
				if (sourceEnforces && targetEnforces) return false;
				if (sourceEnforces && pin.data_type !== IVariableType.Generic)
					return false;
				if (targetEnforces && pinDataType !== IVariableType.Generic)
					return false;
			}
		}
		if (
			pin.data_type === IVariableType.Generic &&
			pinDataType !== IVariableType.Execution
		)
			return true;
		if (
			pinDataType === IVariableType.Generic &&
			pin.data_type !== IVariableType.Execution
		)
			return true;
		return pin.data_type === pinDataType;
	});

	const [sourcePin, sourceNode] = pinCache.get(droppedPin.id) || [];
	if (!sourcePin || !sourceNode || !pin) return;

	const command = connectPinsCommand({
		from_node: droppedPin.pin_type === "Output" ? sourceNode.id : newLayer.id,
		from_pin: droppedPin.pin_type === "Output" ? sourcePin.id : pin.id,
		to_node: droppedPin.pin_type === "Input" ? sourceNode.id : newLayer.id,
		to_pin: droppedPin.pin_type === "Input" ? sourcePin.id : pin.id,
	});

	await executeCommand(command);
}

interface HandleConnectionParams {
	params: any;
	version: [number, number, number] | undefined;
	boardNodes: Record<string, INode>;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
	executeCommand: (command: any) => Promise<any>;
	addEdge: (params: any, edges: any[]) => any[];
	currentEdges: any[];
}

export function handleConnection({
	params,
	version,
	boardNodes,
	pinCache,
	executeCommand,
	addEdge,
	currentEdges,
}: HandleConnectionParams): any[] {
	// Don't execute commands when viewing an old version
	if (typeof version !== "undefined") {
		return currentEdges;
	}

	const isRefInConnection =
		params.sourceHandle?.startsWith("ref_in_") ||
		params.targetHandle?.startsWith("ref_in_");
	const isRefOutConnection =
		params.sourceHandle?.startsWith("ref_out_") ||
		params.targetHandle?.startsWith("ref_out_");

	if (isRefInConnection && isRefOutConnection) {
		handleFunctionReferenceEdgeConnection(params, boardNodes, executeCommand);
		return addEdge(params, currentEdges);
	}

	handleRegularConnection(params, pinCache, executeCommand);
	return addEdge(params, currentEdges);
}

function handleFunctionReferenceEdgeConnection(
	params: any,
	boardNodes: Record<string, INode>,
	executeCommand: (command: any) => Promise<any>,
) {
	const refOutHandle = params.sourceHandle?.startsWith("ref_out_")
		? params.sourceHandle
		: params.targetHandle;
	const refInHandle = params.sourceHandle?.startsWith("ref_in_")
		? params.sourceHandle
		: params.targetHandle;

	const refOutNodeId = refOutHandle.replace("ref_out_", "");
	const refInNodeId = refInHandle.replace("ref_in_", "");

	const refOutNode = boardNodes[refOutNodeId];

	if (refOutNode) {
		const currentRefs = refOutNode.fn_refs?.fn_refs ?? [];
		const updatedRefs = Array.from(new Set([...currentRefs, refInNodeId]));

		const updatedNode = {
			...refOutNode,
			fn_refs: {
				...refOutNode.fn_refs,
				fn_refs: updatedRefs,
				can_reference_fns: refOutNode.fn_refs?.can_reference_fns ?? false,
				can_be_referenced_by_fns:
					refOutNode.fn_refs?.can_be_referenced_by_fns ?? false,
			},
		};

		const command = updateNodeCommand({ node: updatedNode });
		executeCommand(command);
	}
}

function handleRegularConnection(
	params: any,
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>,
	executeCommand: (command: any) => Promise<any>,
) {
	const [sourcePin, sourceNode] = pinCache.get(params.sourceHandle) || [];
	const [targetPin, targetNode] = pinCache.get(params.targetHandle) || [];

	if (!sourcePin || !targetPin || !sourceNode || !targetNode) return;

	const command = connectPinsCommand({
		from_node: sourceNode.id,
		from_pin: sourcePin.id,
		to_node: targetNode.id,
		to_pin: targetPin.id,
	});

	executeCommand(command);
}

interface FunctionReferenceEdgeLike {
	id?: string;
	source?: string;
	sourceHandle?: string | null;
	target?: string;
	targetHandle?: string | null;
	data?: {
		isFnRef?: boolean;
		[key: string]: any;
	};
}

interface FunctionReferenceNodeIds {
	refOutNodeId: string;
	refInNodeId: string;
}

function nodeIdFromRefHandle(
	handleId: string | null | undefined,
	prefix: "ref_out_" | "ref_in_",
): string | undefined {
	if (!handleId?.startsWith(prefix)) return undefined;
	const nodeId = handleId.slice(prefix.length);
	return nodeId.length > 0 ? nodeId : undefined;
}

function functionReferenceNodeIdsFromEdgeId(
	edgeId: string | undefined,
): FunctionReferenceNodeIds | undefined {
	if (!edgeId) return undefined;

	const outToInSeparator = "-ref_in_";
	if (edgeId.startsWith("ref_out_")) {
		const separatorIndex = edgeId.indexOf(outToInSeparator);
		if (separatorIndex !== -1) {
			const refOutNodeId = edgeId.slice("ref_out_".length, separatorIndex);
			const refInNodeId = edgeId.slice(
				separatorIndex + outToInSeparator.length,
			);
			if (refOutNodeId && refInNodeId) {
				return { refOutNodeId, refInNodeId };
			}
		}
	}

	const inToOutSeparator = "-ref_out_";
	if (edgeId.startsWith("ref_in_")) {
		const separatorIndex = edgeId.indexOf(inToOutSeparator);
		if (separatorIndex !== -1) {
			const refInNodeId = edgeId.slice("ref_in_".length, separatorIndex);
			const refOutNodeId = edgeId.slice(
				separatorIndex + inToOutSeparator.length,
			);
			if (refOutNodeId && refInNodeId) {
				return { refOutNodeId, refInNodeId };
			}
		}
	}

	return undefined;
}

export function getFunctionReferenceNodeIdsFromEdge(
	edge: FunctionReferenceEdgeLike,
): FunctionReferenceNodeIds | undefined {
	const sourceRefOutNodeId = nodeIdFromRefHandle(edge.sourceHandle, "ref_out_");
	const sourceRefInNodeId = nodeIdFromRefHandle(edge.sourceHandle, "ref_in_");
	const targetRefOutNodeId = nodeIdFromRefHandle(edge.targetHandle, "ref_out_");
	const targetRefInNodeId = nodeIdFromRefHandle(edge.targetHandle, "ref_in_");

	if (sourceRefOutNodeId && targetRefInNodeId) {
		return {
			refOutNodeId: sourceRefOutNodeId,
			refInNodeId: targetRefInNodeId,
		};
	}

	if (targetRefOutNodeId && sourceRefInNodeId) {
		return {
			refOutNodeId: targetRefOutNodeId,
			refInNodeId: sourceRefInNodeId,
		};
	}

	const nodeIdsFromEdgeId = functionReferenceNodeIdsFromEdgeId(edge.id);
	if (nodeIdsFromEdgeId) return nodeIdsFromEdgeId;

	if (edge.data?.isFnRef && edge.source && edge.target) {
		return {
			refOutNodeId: edge.source,
			refInNodeId: edge.target,
		};
	}

	return undefined;
}

export function removeFunctionReferenceCommandForEdge({
	edge,
	boardNodes,
	skippedNodeIds,
}: {
	edge: FunctionReferenceEdgeLike;
	boardNodes: Record<string, INode> | undefined;
	skippedNodeIds?: Set<string>;
}): any | undefined {
	const nodeIds = getFunctionReferenceNodeIdsFromEdge(edge);
	if (!nodeIds) return undefined;

	const { refOutNodeId, refInNodeId } = nodeIds;
	if (skippedNodeIds?.has(refOutNodeId) || skippedNodeIds?.has(refInNodeId)) {
		return undefined;
	}

	const refOutNode = boardNodes?.[refOutNodeId];
	if (!refOutNode) return undefined;

	const currentRefs = refOutNode.fn_refs?.fn_refs ?? [];
	const updatedRefs = currentRefs.filter((ref: string) => ref !== refInNodeId);
	if (updatedRefs.length === currentRefs.length) return undefined;

	const updatedNode = {
		...refOutNode,
		fn_refs: {
			...refOutNode.fn_refs,
			fn_refs: updatedRefs,
			can_reference_fns: refOutNode.fn_refs?.can_reference_fns ?? false,
			can_be_referenced_by_fns:
				refOutNode.fn_refs?.can_be_referenced_by_fns ?? false,
		},
	};

	return updateNodeCommand({
		node: updatedNode,
	});
}

interface HandleNodesChangeParams {
	changes: any[];
	currentNodes: any[];
	selected: React.MutableRefObject<Set<string>>;
	version: [number, number, number] | undefined;
	boardData: any;
	deletingNodesRef: React.MutableRefObject<Set<string>>;
	executeCommands: (commands: any[]) => Promise<any>;
	applyNodeChanges: (changes: any[], nodes: any[]) => any[];
}

interface Wire {
	id: string;
	sourceHandle?: string | null;
	targetHandle?: string | null;
}

/** A dropped connection whose save is in flight. */
export interface PendingEdgeEdit<T extends Wire = Wire> {
	/** The wire to draw until the saved board carries it. */
	add?: T;
	/** Keys of the wires it moves, replaces or removes, hidden until then. */
	remove: ReadonlySet<string>;
	/** Saved; held only until the parsed board shows the result. */
	settled?: boolean;
}

/** Identifies a wire by its two pins; a layer boundary can list them in either order. */
export function wireKey(wire: Omit<Wire, "id">): string {
	return [wire.sourceHandle ?? "", wire.targetHandle ?? ""]
		.sort()
		.join("\u0000");
}

/** Draws connections that are still saving, so a dropped wire never waits for the backend. */
export function withPendingEdges<T extends Wire>(
	edges: T[],
	pending: readonly PendingEdgeEdit<T>[],
): T[] {
	if (!pending.length) return edges;
	const added = new Map<string, T>();
	const removed = new Set<string>();
	for (const edit of pending) {
		for (const key of edit.remove) {
			removed.add(key);
			added.delete(key);
		}
		if (!edit.add) continue;
		const key = wireKey(edit.add);
		added.set(key, edit.add);
		removed.delete(key);
	}
	const kept = edges.filter((edge) => !removed.has(wireKey(edge)));
	for (const edge of kept) added.delete(wireKey(edge));
	return kept.length === edges.length && !added.size
		? edges
		: [...kept, ...added.values()];
}

/** Whether the parsed board already shows an edit, so nothing is left to hold. */
export function edgeEditApplied<T extends Wire>(
	edit: PendingEdgeEdit<T>,
	edges: readonly T[],
): boolean {
	const keys = new Set(edges.map(wireKey));
	return (
		(!edit.add || keys.has(wireKey(edit.add))) &&
		[...edit.remove].every((key) => !keys.has(key))
	);
}

/**
 * What a dropped connection changes on the canvas before its save lands: the new wire,
 * and the wires it moves or displaces. No connection means the old wire is removed.
 */
export function pendingEdgeEdit({
	connection,
	oldEdge,
	boardNodes,
	pinCache,
	refs,
	pathType,
}: {
	connection?: Connection;
	oldEdge?: Edge;
	boardNodes: Record<string, INode>;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
	refs: Record<string, string>;
	pathType?: string;
}): PendingEdgeEdit<Edge> | undefined {
	const commands = connection
		? buildConnectionCommands({
				connection,
				oldEdge,
				boardNodes,
				pinCache,
				refs,
			})
		: [];
	// A stale or unchanged drop saves nothing, so it shows nothing.
	if (connection && !commands?.length) return undefined;
	const remove = new Set<string>();
	if (oldEdge) remove.add(wireKey(oldEdge));
	for (const command of commands ?? [])
		if (command.command_type === ICommandType.DisconnectPin)
			remove.add(
				wireKey({
					sourceHandle: command.from_pin,
					targetHandle: command.to_pin,
				}),
			);
	const add = connection && pendingEdge(connection, pinCache, pathType);
	if (add) remove.delete(wireKey(add));
	return add || remove.size ? { add, remove } : undefined;
}

export interface UnconfirmedMove {
	position: { x: number; y: number };
	/** The save that carries this position; a later move of the same node replaces it. */
	save: number;
}

/**
 * Board nodes parsed from a revision that may predate the user's latest moves. Nodes being
 * dragged and nodes whose move is still saving keep their local position, so a save that
 * lands in between does not snap them back.
 */
export function keepUnconfirmedPositions<
	T extends {
		id: string;
		position: { x: number; y: number };
		dragging?: boolean;
	},
>(
	parsed: T[],
	current: readonly T[],
	unconfirmed: ReadonlyMap<string, UnconfirmedMove>,
): T[] {
	const local = new Map<string, { x: number; y: number }>();
	const dragging = new Set<string>();
	for (const node of current)
		if (node.dragging) {
			local.set(node.id, node.position);
			dragging.add(node.id);
		}
	for (const [id, move] of unconfirmed)
		if (!local.has(id)) local.set(id, move.position);
	if (!local.size) return parsed;
	return parsed.map((node) => {
		const position = local.get(node.id);
		if (!position) return node;
		const kept = dragging.has(node.id);
		if (
			position.x === node.position.x &&
			position.y === node.position.y &&
			!!node.dragging === kept
		)
			return node;
		return { ...node, position, dragging: kept || undefined };
	});
}

export function handleNodesChange({
	changes,
	currentNodes,
	selected,
	version,
	boardData,
	deletingNodesRef,
	executeCommands,
	applyNodeChanges,
}: HandleNodesChangeParams): any[] {
	if (!changes) return applyNodeChanges(changes, currentNodes);

	const selectChanges = changes.filter(
		(change: any) => change.type === "select",
	);
	for (const change of selectChanges) {
		const selectedId = change.id;
		if (change.selected) selected.current.add(selectedId);
		if (!change.selected) selected.current.delete(selectedId);
	}

	if (typeof version !== "undefined") {
		return applyNodeChanges(changes, currentNodes);
	}

	const removeChanges = changes.filter(
		(change: any) => change.type === "remove",
	);

	if (removeChanges.length > 0) {
		for (const change of removeChanges) {
			const foundNode = Object.values(boardData?.nodes || {}).find(
				(node: any) => node.id === change.id,
			);
			if (foundNode) {
				deletingNodesRef.current.add((foundNode as any).id);
			}
		}

		executeCommands(
			removeChanges
				.map((change) => {
					const foundNode = Object.values(boardData?.nodes || {}).find(
						(node: any) => node.id === change.id,
					);
					if (foundNode) {
						return removeNodeCommand({
							node: foundNode as any,
							connected_nodes: [],
						});
					}

					const foundComment = Object.values(boardData?.comments || {}).find(
						(comment: any) => comment.id === change.id,
					);
					if (foundComment) {
						return removeCommentCommand({
							comment: foundComment as any,
						});
					}

					const foundLayer = Object.values(boardData?.layers || {}).find(
						(layer: any) => layer.id === change.id,
					);
					if (foundLayer) {
						return removeLayerCommand({
							child_layers: [],
							layer: foundLayer as any,
							layer_nodes: [],
							layers: [],
							nodes: [],
							preserve_nodes: false,
						});
					}

					return undefined;
				})
				.filter((command) => command !== undefined) as any[],
		).finally(() => {
			deletingNodesRef.current.clear();
		});
	}

	const nonRemoveChanges = changes.filter(
		(change: any) => change.type !== "remove",
	);
	return applyNodeChanges(nonRemoveChanges, currentNodes);
}

interface HandleEdgesChangeParams {
	changes: any[];
	currentEdges: any[];
	selected: React.MutableRefObject<Set<string>>;
	version: [number, number, number] | undefined;
	boardData: any;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
	deletingNodesRef: React.MutableRefObject<Set<string>>;
	executeCommands: (commands: any[]) => Promise<any>;
	applyEdgeChanges: (changes: any[], edges: any[]) => any[];
}

function isHandleOwnedByDeletingNode(
	handleId: string | undefined,
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>,
	deletingNodesRef: React.MutableRefObject<Set<string>>,
): boolean {
	if (!handleId) {
		return false;
	}

	if (handleId.startsWith("ref_in_")) {
		return deletingNodesRef.current.has(handleId.replace("ref_in_", ""));
	}

	if (handleId.startsWith("ref_out_")) {
		return deletingNodesRef.current.has(handleId.replace("ref_out_", ""));
	}

	const [, pinOwner] = pinCache.get(handleId) || [];
	return pinOwner ? deletingNodesRef.current.has(pinOwner.id) : false;
}

export function handleEdgesChange({
	changes,
	currentEdges,
	selected,
	version,
	boardData,
	pinCache,
	deletingNodesRef,
	executeCommands,
	applyEdgeChanges,
}: HandleEdgesChangeParams): any[] {
	if (!changes || changes.length === 0)
		return applyEdgeChanges(changes, currentEdges);

	const selectChanges = changes.filter(
		(change: any) => change.type === "select",
	);
	for (const change of selectChanges) {
		const selectedId = change.id;
		if (change.selected) selected.current.add(selectedId);
		if (!change.selected) selected.current.delete(selectedId);
	}

	if (typeof version !== "undefined") {
		return applyEdgeChanges(changes, currentEdges);
	}

	const removeChanges = changes.filter(
		(change: any) => change.type === "remove",
	);

	if (removeChanges.length > 0) {
		executeCommands(
			removeChanges
				.map((change: any) => {
					const selectedId = change.id;
					const selectedEdge: FunctionReferenceEdgeLike | undefined =
						currentEdges.find((edge) => edge.id === selectedId);
					const [fallbackFromPinId, fallbackToPinId] = selectedId.split("-");
					const fromPinId = selectedEdge?.sourceHandle ?? fallbackFromPinId;
					const toPinId = selectedEdge?.targetHandle ?? fallbackToPinId;

					if (
						isHandleOwnedByDeletingNode(
							fromPinId,
							pinCache,
							deletingNodesRef,
						) ||
						isHandleOwnedByDeletingNode(toPinId, pinCache, deletingNodesRef)
					) {
						return undefined;
					}

					const functionReferenceEdge = selectedEdge ?? {
						id: selectedId,
						sourceHandle: fromPinId,
						targetHandle: toPinId,
					};
					if (getFunctionReferenceNodeIdsFromEdge(functionReferenceEdge)) {
						return removeFunctionReferenceCommandForEdge({
							edge: functionReferenceEdge,
							boardNodes: boardData?.nodes,
							skippedNodeIds: deletingNodesRef.current,
						});
					}

					if (!fromPinId || !toPinId) {
						return undefined;
					}

					const [fromPin, fromNode] = pinCache.get(fromPinId) || [];
					const [toPin, toNode] = pinCache.get(toPinId) || [];

					if (!fromPin || !toPin || !fromNode || !toNode) return undefined;

					return disconnectPinsCommand({
						from_node: fromNode.id,
						from_pin: fromPin.id,
						to_node: toNode.id,
						to_pin: toPin.id,
					});
				})
				.filter((command: any) => command !== undefined) as any[],
		);
	}

	const nonRemoveChanges = changes.filter(
		(change: any) => change.type !== "remove",
	);
	return applyEdgeChanges(nonRemoveChanges, currentEdges);
}
