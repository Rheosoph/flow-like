import { createId } from "@paralleldrive/cuid2";
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
	executeCommand: (command: any, refetch?: boolean) => Promise<any>;
}

export async function handlePlaceNode({
	node,
	position,
	droppedPin,
	currentLayer,
	refs,
	boardNodes,
	pinCache,
	executeCommand,
}: PlaceNodeParams) {
	const result = addNodeCommand({
		node: { ...node, coordinates: [position.x, position.y, 0] },
		current_layer: currentLayer,
	});

	await executeCommand(result.command);
	const new_node = result.node;

	if (!droppedPin) return;

	const isRefInHandle = droppedPin.id.startsWith("ref_in_");
	const isRefOutHandle = droppedPin.id.startsWith("ref_out_");

	if (isRefInHandle || isRefOutHandle) {
		await handleFunctionReferenceConnection({
			droppedPin,
			newNode: new_node,
			isRefOutHandle,
			boardNodes,
			executeCommand,
		});
	} else {
		await handleRegularPinConnection({
			droppedPin,
			newNode: new_node,
			refs,
			pinCache,
			executeCommand,
		});
	}
}

interface FunctionReferenceConnectionParams {
	droppedPin: IPin;
	newNode: INode;
	isRefOutHandle: boolean;
	boardNodes: Record<string, INode>;
	executeCommand: (command: any) => Promise<any>;
}

async function handleFunctionReferenceConnection({
	droppedPin,
	newNode,
	isRefOutHandle,
	boardNodes,
	executeCommand,
}: FunctionReferenceConnectionParams) {
	const sourceNodeId = isRefOutHandle
		? droppedPin.id.replace("ref_out_", "")
		: droppedPin.id.replace("ref_in_", "");

	const sourceNode = boardNodes[sourceNodeId];
	if (!sourceNode) return;

	if (isRefOutHandle) {
		// ref_out was dropped: source node references the new node
		const currentRefs = sourceNode.fn_refs?.fn_refs ?? [];
		const updatedRefs = Array.from(new Set([...currentRefs, newNode.id]));

		const updatedNode = {
			...sourceNode,
			fn_refs: {
				...sourceNode.fn_refs,
				fn_refs: updatedRefs,
				can_reference_fns: sourceNode.fn_refs?.can_reference_fns ?? false,
				can_be_referenced_by_fns:
					sourceNode.fn_refs?.can_be_referenced_by_fns ?? false,
			},
		};

		const command = updateNodeCommand({ node: updatedNode });
		await executeCommand(command);
	} else {
		// ref_in was dropped: new node references the source node
		const currentRefs = newNode.fn_refs?.fn_refs ?? [];
		const updatedRefs = Array.from(new Set([...currentRefs, sourceNodeId]));

		const updatedNode = {
			...newNode,
			fn_refs: {
				...newNode.fn_refs,
				fn_refs: updatedRefs,
				can_reference_fns: newNode.fn_refs?.can_reference_fns ?? false,
				can_be_referenced_by_fns:
					newNode.fn_refs?.can_be_referenced_by_fns ?? false,
			},
		};

		const command = updateNodeCommand({ node: updatedNode });
		await executeCommand(command);
	}
}

interface RegularPinConnectionParams {
	droppedPin: IPin;
	newNode: INode;
	refs: Record<string, any>;
	pinCache: Map<string, [IPin, INode | ILayer, boolean]>;
	executeCommand: (command: any) => Promise<any>;
}

async function handleRegularPinConnection({
	droppedPin,
	newNode,
	refs,
	pinCache,
	executeCommand,
}: RegularPinConnectionParams) {
	const pinType = droppedPin.pin_type === "Input" ? "Output" : "Input";
	const pinValueType = droppedPin.value_type;
	const pinDataType = droppedPin.data_type;
	const schema = refs?.[droppedPin.schema ?? ""] ?? droppedPin.schema;
	const options = droppedPin.options;

	const pin = findMatchingPin(newNode.pins, {
		pinType,
		pinValueType,
		pinDataType,
		schema,
		options,
		refs,
	});

	const [sourcePin, sourceNode] = pinCache.get(droppedPin.id) || [];
	if (!sourcePin || !sourceNode || !pin) return;

	const command = connectPinsCommand({
		from_node: droppedPin.pin_type === "Output" ? sourceNode.id : newNode.id,
		from_pin: droppedPin.pin_type === "Output" ? sourcePin.id : pin.id,
		to_node: droppedPin.pin_type === "Input" ? sourceNode.id : newNode.id,
		to_pin: droppedPin.pin_type === "Input" ? sourcePin.id : pin.id,
	});

	await executeCommand(command);
}

interface FindMatchingPinParams {
	pinType: string;
	pinValueType: string;
	pinDataType: string;
	schema: any;
	options: any;
	refs: Record<string, any>;
}

function findMatchingPin(
	pins: Record<string, IPin>,
	{
		pinType,
		pinValueType,
		pinDataType,
		schema,
		options,
		refs,
	}: FindMatchingPinParams,
): IPin | undefined {
	return Object.values(pins).find((pin) => {
		if (typeof schema === "string" || typeof pin.schema === "string") {
			const pinSchema = refs?.[pin.schema ?? ""] ?? pin.schema;
			if (
				(pin.options?.enforce_schema || options?.enforce_schema) &&
				schema !== pinSchema &&
				pin.data_type !== IVariableType.Generic &&
				pinDataType !== IVariableType.Generic
			)
				return false;
		}
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

	let edges = currentEdges;

	const selectChanges = changes.filter(
		(change: any) => change.type === "select",
	);
	for (const change of selectChanges) {
		const selectedId = change.id;
		const selectedEdge: any = edges.find((edge) => edge.id === selectedId);

		if (change.selected) selected.current.add(selectedId);
		if (!change.selected) selected.current.delete(selectedId);

		if (selectedEdge?.data_type !== "Execution") {
			edges = edges.map((edge) =>
				edge.id === selectedId ? { ...edge, animated: !change.selected } : edge,
			);
		}
	}

	if (typeof version !== "undefined") {
		return applyEdgeChanges(changes, edges);
	}

	const removeChanges = changes.filter(
		(change: any) => change.type === "remove",
	);

	if (removeChanges.length > 0) {
		executeCommands(
			removeChanges
				.map((change: any) => {
					const selectedId = change.id;
					const [fromPinId, toPinId] = selectedId.split("-");

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

					const isRefInConnection =
						fromPinId?.startsWith("ref_in_") || toPinId?.startsWith("ref_in_");
					const isRefOutConnection =
						fromPinId?.startsWith("ref_out_") ||
						toPinId?.startsWith("ref_out_");

					if (isRefInConnection && isRefOutConnection) {
						const refOutHandle = fromPinId?.startsWith("ref_out_")
							? fromPinId
							: toPinId;
						const refInHandle = fromPinId?.startsWith("ref_in_")
							? fromPinId
							: toPinId;

						const refOutNodeId = refOutHandle.replace("ref_out_", "");
						const refInNodeId = refInHandle.replace("ref_in_", "");

						if (
							deletingNodesRef.current.has(refOutNodeId) ||
							deletingNodesRef.current.has(refInNodeId)
						) {
							return undefined;
						}

						const refOutNode = boardData?.nodes[refOutNodeId];
						if (refOutNode) {
							const currentRefs = refOutNode.fn_refs?.fn_refs ?? [];
							const updatedRefs = currentRefs.filter(
								(ref: string) => ref !== refInNodeId,
							);

							const updatedNode = {
								...refOutNode,
								fn_refs: {
									...refOutNode.fn_refs,
									fn_refs: updatedRefs,
									can_reference_fns:
										refOutNode.fn_refs?.can_reference_fns ?? false,
									can_be_referenced_by_fns:
										refOutNode.fn_refs?.can_be_referenced_by_fns ?? false,
								},
							};

							return updateNodeCommand({
								node: updatedNode,
							});
						}
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
	return applyEdgeChanges(nonRemoveChanges, edges);
}
