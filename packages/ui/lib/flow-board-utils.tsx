import { createId } from "@paralleldrive/cuid2";
import { CopyIcon, Import } from "lucide-react";
import type { RefObject } from "react";
import { toast } from "sonner";
import type { FlowSelectorDataRef } from "../components/flow/flow-selector-data";
import { InnerLayerNodeType } from "../components/flow/layer-inner-node";
import { typeToColor } from "../components/flow/utils";
import {
	copyPasteCommand,
	removeLayerCommand,
	upsertCommentCommand,
	upsertLayerCommand,
} from "./command/generic-command";
import { detectFormat } from "./importer/detect";
import { translateDify } from "./importer/dify-translator";
import { translateN8n } from "./importer/n8n-translator";
import type { DifyWorkflow, N8nWorkflow } from "./importer/types";
import { toastSuccess } from "./messages";
import { isWebkitLite } from "./platform";
import type { IGenericCommand, IValueType, IVariable } from "./schema";
import {
	type IBoard,
	type IComment,
	ICommentType,
	IExecutionMode,
	type ILayer,
	ILayerType,
} from "./schema/flow/board";
import { IVariableType } from "./schema/flow/node";
import type { IFnRefs, INode } from "./schema/flow/node";
import { type IPin, type IPinOptions, IPinType } from "./schema/flow/pin";
import { parseUint8ArrayToJson } from "./uint8";

export function hexToRgba(hex: string, alpha = 0.3): string {
	let c = hex.replace("#", "");
	if (c.length === 3) c = c[0] + c[0] + c[1] + c[1] + c[2] + c[2];
	const num = Number.parseInt(c, 16);
	const r = (num >> 16) & 255;
	const g = (num >> 8) & 255;
	const b = num & 255;
	return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export function normalizeSelectionNodes(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value.filter(
		(nodeId: unknown): nodeId is string => typeof nodeId === "string",
	);
}

function boardDataVersion(board: IBoard): string {
	const updatedAt = board.updated_at
		? `${board.updated_at.secs_since_epoch}.${board.updated_at.nanos_since_epoch}`
		: "";
	return [
		board.id,
		board.version?.join(".") ?? "",
		updatedAt,
		Object.keys(board.nodes ?? {}).length,
		Object.keys(board.layers ?? {}).length,
		Object.keys(board.variables ?? {}).length,
		Object.keys(board.refs ?? {}).length,
	].join(":");
}

function renderedNodeCacheKey(id: string, hash: unknown): string {
	return `${id}:${String(hash)}`;
}

interface ISerializedPin {
	id: string;
	name: string;
	friendly_name: string;
	pin_type: IPinType;
	data_type: IVariableType;
	value_type: IValueType;
	depends_on: string[];
	connected_to: string[];
	default_value?: number[];
	index: number;
	schema?: string | null;
	options?: IPinOptions | null;
}
interface ISerializedNode {
	id: string;
	name: string;
	friendly_name: string;
	comment?: string;
	coordinates?: number[];
	pins: {
		[key: string]: ISerializedPin;
	};
	layer?: string;
	fn_refs?: IFnRefs;
	version?: number;
}

function serializeNode(node: INode): ISerializedNode {
	const pins: {
		[key: string]: ISerializedPin;
	} = {};

	for (const pin of Object.values(node.pins)) {
		pins[pin.id] = {
			id: pin.id,
			name: pin.name,
			friendly_name: pin.friendly_name,
			pin_type: pin.pin_type,
			data_type: pin.data_type,
			value_type: pin.value_type,
			depends_on: pin.depends_on,
			connected_to: pin.connected_to,
			default_value: pin.default_value ?? undefined,
			index: pin.index,
			schema: pin.schema ?? undefined,
			options: pin.options ?? undefined,
		};
	}

	return {
		id: node.id,
		name: node.name,
		friendly_name: node.friendly_name,
		comment: node.comment ?? undefined,
		coordinates: node.coordinates ?? undefined,
		pins: pins,
		layer: node.layer ?? undefined,
		fn_refs: node.fn_refs ?? undefined,
		version: node.version ?? undefined,
	};
}

function deserializeNode(node: ISerializedNode): INode {
	const pins: {
		[key: string]: IPin;
	} = {};

	for (const pin of Object.values(node.pins)) {
		pins[pin.id] = {
			id: pin.id,
			name: pin.name,
			friendly_name: pin.friendly_name,
			pin_type: pin.pin_type,
			data_type: pin.data_type,
			value_type: pin.value_type,
			depends_on: pin.depends_on,
			connected_to: pin.connected_to,
			default_value: pin.default_value ?? undefined,
			index: pin.index,
			description: "",
			schema: pin.schema || undefined,
			options: pin.options ?? undefined,
		};
	}

	return {
		id: node.id,
		category: "",
		name: node.name,
		description: "",
		friendly_name: node.friendly_name,
		coordinates: node.coordinates ?? [0, 0, 0],
		comment: node.comment ?? "",
		pins: pins,
		layer: node.layer ?? "",
		fn_refs: node.fn_refs ?? undefined,
		version: node.version ?? undefined,
	};
}

function isEditableClipboardTarget(
	element: EventTarget | Element | null,
): boolean {
	if (!(element instanceof Element)) return false;
	if (element instanceof HTMLElement && element.isContentEditable) {
		return true;
	}
	if (
		element instanceof HTMLInputElement ||
		element instanceof HTMLTextAreaElement ||
		element instanceof HTMLSelectElement
	) {
		return true;
	}
	return Boolean(
		element.closest("[contenteditable='true'], [contenteditable='']"),
	);
}

function hasSelectedPageText(): boolean {
	const selection = window.getSelection?.();
	return Boolean(selection && !selection.isCollapsed && selection.toString());
}

export function shouldIgnoreBoardClipboardEvent(
	event?: ClipboardEvent,
): boolean {
	return (
		isEditableClipboardTarget(event?.target ?? null) ||
		isEditableClipboardTarget(document.activeElement) ||
		hasSelectedPageText()
	);
}

export function isValidConnection(
	connection: any,
	cache: Map<string, [IPin, INode | ILayer, boolean]>,
	refs: { [key: string]: string },
) {
	const refIn =
		connection.sourceHandle.startsWith("ref_in_") ||
		connection.targetHandle.startsWith("ref_in_");
	const refOut =
		connection.sourceHandle.startsWith("ref_out_") ||
		connection.targetHandle.startsWith("ref_out_");

	if (refIn || refOut) {
		return refIn && refOut;
	}

	const [sourcePin, sourceNode] = cache.get(connection.sourceHandle) || [];
	const [targetPin, targetNode] = cache.get(connection.targetHandle) || [];

	if (!sourcePin || !targetPin) {
		console.warn(
			`Invalid connection: source or target pin not found for ${connection.sourceHandle} or ${connection.targetHandle}`,
		);
		return false;
	}
	if (!sourceNode || !targetNode) {
		console.warn(
			`Invalid connection: source or target node not found for ${connection.sourceHandle} or ${connection.targetHandle}`,
		);
		return false;
	}

	if (sourceNode.id === targetNode.id) {
		console.warn(
			`Invalid connection: source and target nodes are the same (${sourceNode.id})`,
		);
		return false;
	}

	return doPinsMatch(sourcePin, targetPin, refs, sourceNode, targetNode);
}

function invertPinType(type: IPinType): IPinType {
	return type === IPinType.Input ? IPinType.Output : IPinType.Input;
}

function stripCallFunctionRef(node: INode): {
	node: INode;
	functionLayerId: string | undefined;
} {
	const layerPin = Object.values(node.pins).find(
		(p) => p.name === "function_layer_id",
	);
	const functionLayerId = layerPin?.default_value
		? parseUint8ArrayToJson(layerPin.default_value)
		: undefined;
	return { node, functionLayerId };
}

/** Prefix for break struct field pins */
const BREAK_STRUCT_PIN_PREFIX = "__break_struct_field__";
/** Prefix for make struct field pins */
const MAKE_STRUCT_PIN_PREFIX = "__make_struct_field__";

/**
 * Check if a pin is a break/make struct field pin.
 * These pins have special connection rules for schema matching.
 */
function isStructFieldPin(pin: IPin): boolean {
	return (
		pin.name.startsWith(BREAK_STRUCT_PIN_PREFIX) ||
		pin.name.startsWith(MAKE_STRUCT_PIN_PREFIX)
	);
}

/**
 * Check if this is a struct_in or struct_out pin from break/make struct nodes.
 */
function isStructIOPin(pin: IPin): boolean {
	return pin.name === "struct_in" || pin.name === "struct_out";
}

export function doPinsMatch(
	sourcePin: IPin,
	targetPin: IPin,
	refs: { [key: string]: string },
	sourceNode?: INode | ILayer,
	targetNode?: INode | ILayer,
) {
	if (sourceNode?.id.endsWith("-return")) {
		sourcePin.pin_type = invertPinType(sourcePin.pin_type);
	}

	if (sourceNode?.id.endsWith("-input")) {
		sourcePin.pin_type = invertPinType(sourcePin.pin_type);
	}

	if (targetNode?.id.endsWith("-return")) {
		targetPin.pin_type = invertPinType(targetPin.pin_type);
	}

	if (targetNode?.id.endsWith("-input")) {
		targetPin.pin_type = invertPinType(targetPin.pin_type);
	}

	if (
		(sourcePin.name === "route_in" &&
			sourcePin.data_type === IVariableType.Generic) ||
		(targetPin.name === "route_in" &&
			targetPin.data_type === IVariableType.Generic)
	)
		return true;
	if (
		(targetPin.name === "route_out" &&
			targetPin.data_type === IVariableType.Generic) ||
		(sourcePin.name === "route_out" &&
			sourcePin.data_type === IVariableType.Generic)
	)
		return true;

	if (sourcePin.pin_type === targetPin.pin_type) {
		console.warn(
			`Invalid connection: source and target pins have the same type (${sourcePin.pin_type})`,
		);
		return false;
	}

	let schemaSource = sourcePin.schema;
	if (schemaSource) {
		schemaSource = refs[schemaSource] ?? schemaSource;
	}

	let schemaTarget = targetPin.schema;
	if (schemaTarget) {
		schemaTarget = refs[schemaTarget] ?? schemaTarget;
	}

	if (sourcePin.schema && targetPin.schema) {
		if (
			schemaSource !== schemaTarget &&
			sourcePin.options?.enforce_schema !== false &&
			targetPin.options?.enforce_schema !== false
		)
			return false;
	}

	if (targetPin.value_type !== sourcePin.value_type) {
		const sourceEnforces =
			sourcePin.options?.enforce_generic_value_type ?? false;
		const targetEnforces =
			targetPin.options?.enforce_generic_value_type ?? false;
		if (sourceEnforces || targetEnforces) {
			if (sourceEnforces && targetEnforces) return false;
			if (sourceEnforces && targetPin.data_type !== "Generic") return false;
			if (targetEnforces && sourcePin.data_type !== "Generic") return false;
		}
	}

	if (
		(sourcePin.data_type === "Generic" || targetPin.data_type === "Generic") &&
		sourcePin.data_type !== "Execution" &&
		targetPin.data_type !== "Execution"
	)
		return true;

	// Special handling for break/make struct I/O pins
	// These pins (struct_in, struct_out) should be able to connect to any struct with a schema
	// The schema will be adopted dynamically via on_update
	if (
		(isStructIOPin(sourcePin) || isStructIOPin(targetPin)) &&
		sourcePin.data_type === IVariableType.Struct &&
		targetPin.data_type === IVariableType.Struct
	) {
		// Allow connection if one side has a schema (the break/make node will adopt it)
		if (sourcePin.schema || targetPin.schema) {
			if (sourcePin.value_type !== targetPin.value_type) return false;
			return true;
		}
	}

	if (
		(targetPin.options?.enforce_schema || sourcePin.options?.enforce_schema) &&
		sourcePin.name !== "value_ref" &&
		targetPin.name !== "value_ref" &&
		sourcePin.name !== "value_in" &&
		targetPin.name !== "value_in" &&
		sourcePin.data_type !== "Generic" &&
		targetPin.data_type !== "Generic"
	) {
		if (!sourcePin.schema || !targetPin.schema) return false;
		if (schemaSource !== schemaTarget) return false;
	}

	if (sourcePin.value_type !== targetPin.value_type) return false;
	if (sourcePin.data_type !== targetPin.data_type) return false;

	return true;
}

export function parseBoard(
	board: IBoard,
	appId: string,
	handleCopy: (event?: ClipboardEvent) => void,
	pushLayer: (layer: ILayer) => void,
	executeBoard: (node: INode, payload?: object) => Promise<void>,
	executeCommand: (command: IGenericCommand, append: boolean) => Promise<any>,
	selected: Set<string>,
	connectionMode?: string,
	oldNodes?: any[],
	oldEdges?: any[],
	currentLayer?: string,
	boardRef?: RefObject<IBoard | undefined>,
	version?: [number, number, number],
	onOpenInfo?: (node: INode) => void,
	onExplain?: (nodeIds: string[]) => void,
	onFilterLogs?: (nodeId: string) => void,
	remoteBoardExecution?: {
		isOffline: boolean;
		onRemoteExecute?: (node: INode, payload?: object) => Promise<void>;
	},
	catalogLookup?: { nodeNames: Set<string>; wasmNodeKeys: Set<string> },
	selectorDataRef?: FlowSelectorDataRef,
	selectorDataVersion?: number,
) {
	const nodes: any[] = [];
	const edges: any[] = [];
	const cache = new Map<string, [IPin, INode | ILayer, boolean]>();
	const oldNodesMap = new Map<string, any>();
	const oldEdgesMap = new Map<string, any>();
	const addedNodeIds = new Set<string>(); // Track which node IDs have been added
	const boardVersionToken = boardDataVersion(board);

	// Compute a hash of all fn_refs to detect changes
	const fnRefsHash = Object.values(board.nodes)
		.map((n) => `${n.id}:${n.fn_refs?.fn_refs?.join(",") ?? ""}`)
		.join(";");

	for (const oldNode of oldNodes ?? []) {
		const hash = oldNode.data?.hash;
		if (typeof oldNode.id === "string" && hash !== undefined && hash !== null) {
			oldNodesMap.set(renderedNodeCacheKey(oldNode.id, hash), oldNode);
		}
	}

	for (const edge of oldEdges ?? []) {
		oldEdgesMap.set(edge.id, edge);
	}

	for (const node of Object.values(board.nodes)) {
		const nodeLayer = (node.layer ?? "") === "" ? undefined : node.layer;
		for (const pin of Object.values(node.pins)) {
			cache.set(pin.id, [pin, node, nodeLayer === currentLayer]);
		}
		if (nodeLayer !== currentLayer) continue;

		// Skip if this node ID has already been added (prevents duplicates)
		if (addedNodeIds.has(node.id)) {
			console.warn(`Duplicate node ID detected: ${node.id}, skipping...`);
			continue;
		}
		addedNodeIds.add(node.id);

		const hash = node.hash ?? -1;
		const canRunWasmOnServer =
			Boolean(node.wasm?.package_id) &&
			!remoteBoardExecution?.isOffline &&
			remoteBoardExecution?.onRemoteExecute !== undefined &&
			board.execution_mode !== IExecutionMode.Local &&
			!node.only_offline;
		const isUnavailable = catalogLookup
			? node.wasm?.package_id
				? !canRunWasmOnServer &&
					!catalogLookup.wasmNodeKeys.has(
						`${node.wasm.package_id}:${node.name}`,
					)
				: !catalogLookup.nodeNames.has(node.name)
			: false;
		const oldNode =
			hash === -1
				? undefined
				: oldNodesMap.get(renderedNodeCacheKey(node.id, hash));
		const sel = selected.has(node.id);
		if (
			oldNode &&
			oldNode.selected === sel &&
			oldNode.data?.isUnavailable === isUnavailable &&
			oldNode.data?.fnRefsHash === fnRefsHash &&
			oldNode.data?.boardDataVersion === boardVersionToken &&
			oldNode.data?.selectorDataRef === selectorDataRef &&
			oldNode.data?.selectorDataVersion === selectorDataVersion
		) {
			// Hash + selected + isUnavailable + fnRefsHash all match — reuse exact reference
			nodes.push(oldNode);
		} else if (oldNode) {
			// Hash matches but some derived state changed — shallow update
			const nodeForData =
				node.name === "control_call_function"
					? stripCallFunctionRef(node).node
					: node;
			nodes.push({
				...oldNode,
				data: {
					...oldNode.data,
					isUnavailable,
					fnRefsHash,
					node: nodeForData,
					boardRef,
					boardDataVersion: boardVersionToken,
					selectorDataRef,
					selectorDataVersion,
				},
				selected: sel,
			});
		} else {
			const isCallFunction = node.name === "control_call_function";
			const { node: nodeForData, functionLayerId } = isCallFunction
				? stripCallFunctionRef(node)
				: { node, functionLayerId: undefined };

			nodes.push({
				id: node.id,
				type: isCallFunction ? "callFunctionNode" : "node",
				zIndex: 20,
				position: {
					x: node.coordinates?.[0] ?? 0,
					y: node.coordinates?.[1] ?? 0,
				},
				data: {
					label: node.name,
					boardRef: boardRef,
					boardDataVersion: boardVersionToken,
					selectorDataRef,
					selectorDataVersion,
					fnRefsHash: fnRefsHash,
					node: nodeForData,
					hash: hash,
					boardId: board.id,
					appId: appId,
					version: version,
					isUnavailable,
					functionLayerId,
					currentLayerId: currentLayer,
					onExecute: async (node: INode, payload?: object) => {
						await executeBoard(node, payload);
					},
					onRemoteExecute: remoteBoardExecution?.onRemoteExecute
						? async (node: INode, payload?: object) => {
								await remoteBoardExecution.onRemoteExecute?.(node, payload);
							}
						: undefined,
					isOffline: remoteBoardExecution?.isOffline ?? true,
					onCopy: async () => {
						handleCopy();
					},
					onOpenInfo: onOpenInfo,
					onExplain: onExplain,
					onFilterLogs: onFilterLogs,
					executionMode: board.execution_mode,
				},
				selected: selected.has(node.id),
			});
		}
	}

	const activeLayer = new Set();
	if (board.layers)
		for (const layer of Object.values(board.layers)) {
			if (layer.type === ILayerType.Function && layer.id !== currentLayer)
				continue;
			const parentLayer =
				(layer.parent_id ?? "") === "" ? undefined : layer.parent_id;
			if (parentLayer !== currentLayer) {
				if (layer.id === currentLayer) {
					// Build immutable inverted pins for the current layer view and split into input/return inner nodes
					const inputNodePins: { [key: string]: IPin } = {};
					const returnNodePins: { [key: string]: IPin } = {};

					for (const pin of Object.values(layer.pins)) {
						const inverted: IPin = {
							...pin,
							pin_type: invertPinType(pin.pin_type),
						};
						// cache the inverted pin with the layer as owner; visibility = true (we are inside this layer)
						cache.set(inverted.id, [inverted, layer, true]);
						// Pins that become Output feed the -input node; those that become Input feed the -return node
						if (inverted.pin_type === IPinType.Output) {
							inputNodePins[inverted.id] = inverted;
						} else {
							returnNodePins[inverted.id] = inverted;
						}
					}

					nodes.push({
						id: layer.id + "-input",
						type: "layerInnerNode",
						position: {
							x: layer.in_coordinates?.[0],
							y: layer.in_coordinates?.[1],
						},
						zIndex: 19,
						data: {
							label: layer.id,
							boardId: board.id,
							appId: appId,
							boardRef: boardRef,
							boardDataVersion: boardVersionToken,
							selectorDataRef,
							selectorDataVersion,
							type: InnerLayerNodeType.INPUT,
							layer: {
								...layer,
								pins: inputNodePins,
							},
							hash: layer.hash ?? -1,
							pushLayer: async (layer: ILayer) => {
								pushLayer(layer);
							},
							onLayerUpdate: async (layer: ILayer) => {
								const command = upsertLayerCommand({
									current_layer: currentLayer,
									layer: layer,
									node_ids: [],
								});
								await executeCommand(command, false);
							},
							onLayerRemove: async (layer: ILayer, preserve_nodes: boolean) => {
								const command = removeLayerCommand({
									layer,
									child_layers: [],
									layer_nodes: [],
									layers: [],
									nodes: [],
									preserve_nodes,
								});
								await executeCommand(command, false);
							},
						},
						selected: selected.has(layer.id + "-input"),
					});
					nodes.push({
						id: layer.id + "-return",
						type: "layerInnerNode",
						position: {
							x: layer.out_coordinates?.[0],
							y: layer.out_coordinates?.[1],
						},
						zIndex: 19,
						data: {
							label: layer.id,
							boardId: board.id,
							appId: appId,
							boardRef: boardRef,
							boardDataVersion: boardVersionToken,
							selectorDataRef,
							selectorDataVersion,
							type: InnerLayerNodeType.RETURN,
							layer: {
								...layer,
								pins: returnNodePins,
							},
							hash: layer.hash ?? -1,
							pushLayer: async (layer: ILayer) => {
								pushLayer(layer);
							},
							onLayerUpdate: async (layer: ILayer) => {
								const command = upsertLayerCommand({
									current_layer: currentLayer,
									layer: layer,
									node_ids: [],
								});
								await executeCommand(command, false);
							},
							onLayerRemove: async (layer: ILayer, preserve_nodes: boolean) => {
								const command = removeLayerCommand({
									layer,
									child_layers: [],
									layer_nodes: [],
									layers: [],
									nodes: [],
									preserve_nodes,
								});
								await executeCommand(command, false);
							},
						},
						selected: selected.has(layer.id + "-return"),
					});
				}

				continue;
			}

			const lookup: Record<string, INode | ILayer> = {};
			if (layer.pins)
				for (const pin of Object.values(layer.pins)) {
					const [_, node] = cache.get(pin.id) ?? [pin.id, layer];
					if (node) lookup[pin.id] = node;
					cache.set(pin.id, [pin, node, true]);
				}

			activeLayer.add(layer.id);
			nodes.push({
				id: layer.id,
				type: "layerNode",
				position: { x: layer.coordinates[0], y: layer.coordinates[1] },
				zIndex: 19,
				data: {
					label: layer.id,
					boardId: board.id,
					appId: appId,
					layer: layer,
					boardRef: boardRef,
					boardDataVersion: boardVersionToken,
					selectorDataRef,
					selectorDataVersion,
					hash: layer.hash ?? -1,
					pinLookup: lookup,
					pushLayer: async (layer: ILayer) => {
						pushLayer(layer);
					},
					onLayerUpdate: async (layer: ILayer) => {
						const command = upsertLayerCommand({
							current_layer: currentLayer,
							layer: layer,
							node_ids: [],
						});
						await executeCommand(command, false);
					},
					onLayerRemove: async (layer: ILayer, preserve_nodes: boolean) => {
						const command = removeLayerCommand({
							layer,
							child_layers: [],
							layer_nodes: [],
							layers: [],
							nodes: [],
							preserve_nodes,
						});
						await executeCommand(command, false);
					},
					onExplain: onExplain,
				},
				selected: selected.has(layer.id),
			});
		}

	// Helper to resolve inner node id for current layer boundary pins
	const resolveInnerNodeId = (layerId: string, pin: IPin) =>
		// Pins were inverted when entering the current layer view:
		// - boundary Input -> inverted Output -> belongs to `${layerId}-input`
		// - boundary Output -> inverted Input -> belongs to `${layerId}-return`
		pin.pin_type === IPinType.Output ? `${layerId}-input` : `${layerId}-return`;

	const currentLayerRef: ILayer | undefined = board.layers[currentLayer ?? ""];
	for (const [pin, node, visible] of cache.values()) {
		if (pin.connected_to.length === 0) continue;

		for (const connectedTo of pin.connected_to) {
			const [conntectedPin, connectedNode, connectedVisible] =
				cache.get(connectedTo) || [];
			const connectedLayer = board.layers[connectedNode?.layer ?? ""];
			if (!visible && !connectedVisible) continue;
			if (!conntectedPin || !connectedNode) continue;

			if (
				visible !== connectedVisible &&
				(connectedLayer?.parent_id ?? "") !== (currentLayer ?? "")
			) {
				if (!visible && node.layer === currentLayerRef?.parent_id) {
					let coordinates = node.coordinates ?? [0, 0, 0];

					if (currentLayerRef?.nodes[node.id]) {
						coordinates = currentLayerRef.nodes[node.id]?.coordinates ?? [
							0, 0, 0,
						];
					}
				} else if (
					!connectedVisible &&
					connectedNode.layer === currentLayerRef?.parent_id
				) {
					let coordinates = connectedNode.coordinates ?? [0, 0, 0];

					if (currentLayerRef?.nodes[connectedNode.id]) {
						coordinates = currentLayerRef.nodes[connectedNode.id]
							?.coordinates ?? [0, 0, 0];
					}
				}
			}

			// Map endpoints:
			// - If the owner is the current layer, route to the inner nodes (-input / -return)
			// - Else, if the owner lives in an active child layer, route to that layer node
			// - Else, route to the node itself
			const sourceNodeId =
				((node as any)?.id ?? "") === (currentLayer ?? "")
					? resolveInnerNodeId(currentLayer!, pin)
					: activeLayer.has((node as any)?.layer ?? "")
						? (node as any).layer
						: (node as any)?.id;

			const targetNodeId =
				((connectedNode as any)?.id ?? "") === (currentLayer ?? "")
					? resolveInnerNodeId(currentLayer!, conntectedPin)
					: activeLayer.has((connectedNode as any)?.layer ?? "")
						? (connectedNode as any).layer
						: (connectedNode as any)?.id;

			const edgeId = `${pin.id}-${connectedTo}`;
			const sel = selected.has(edgeId);
			const oldEdge = oldEdgesMap.get(edgeId);

			if (
				oldEdge &&
				visible === connectedVisible &&
				oldEdge.source === sourceNodeId &&
				oldEdge.target === targetNodeId &&
				oldEdge.selected === sel &&
				oldEdge.data?.pathType === connectionMode
			) {
				edges.push(oldEdge);
				continue;
			}

			if (pin.id && conntectedPin.id)
				edges.push({
					id: `${pin.id}-${conntectedPin.id}`,
					source: sourceNodeId,
					sourceHandle: pin.id,
					zIndex: 18,
					data: {
						fromLayer: (node as any).layer,
						toLayer: (connectedNode as any).layer,
						pathType: connectionMode,
						data_type: pin.data_type,
					},
					animated: !isWebkitLite() && pin.data_type !== "Execution",
					reconnectable: true,
					target: targetNodeId,
					targetHandle: conntectedPin.id,
					style: { stroke: typeToColor(pin.data_type) },
					type: pin.data_type === "Execution" ? "execution" : "data",
					data_type: pin.data_type,
					selected: sel,
				});
			else {
				console.log(`${pin.id}-${connectedTo} edge not created`);
			}
		}
	}

	// Create edges for function references
	for (const node of Object.values(board.nodes)) {
		const nodeLayer = (node.layer ?? "") === "" ? undefined : node.layer;
		if (nodeLayer !== currentLayer) continue;

		if (node.fn_refs?.can_reference_fns && node.fn_refs.fn_refs.length > 0) {
			for (const refNodeId of node.fn_refs.fn_refs) {
				const targetNode = board.nodes[refNodeId];
				if (!targetNode) continue;

				const targetLayer =
					(targetNode.layer ?? "") === "" ? undefined : targetNode.layer;
				if (targetLayer !== currentLayer) continue;

				const sourceHandle = `ref_out_${node.id}`;
				const targetHandle = `ref_in_${refNodeId}`;
				const edgeId = `${sourceHandle}-${targetHandle}`;

				const existingEdge = oldEdgesMap.get(edgeId);

				if (existingEdge) {
					edges.push(existingEdge);
				} else {
					edges.push({
						id: edgeId,
						source: node.id,
						sourceHandle: sourceHandle,
						target: refNodeId,
						targetHandle: targetHandle,
						zIndex: 18,
						data: {
							fromLayer: nodeLayer,
							toLayer: targetLayer,
							isFnRef: true,
							pathType: connectionMode,
						},
						animated: !isWebkitLite(),
						reconnectable: true,
						style: {
							stroke: "var(--pin-fn-ref)",
						},
						type: "veil",
						selected: selected.has(edgeId),
					});
				}
			}
		}
	}
	for (const comment of Object.values(board.comments)) {
		const commentLayer =
			(comment.layer ?? "") === "" ? undefined : comment.layer;
		if (commentLayer !== currentLayer) continue;
		const hash = comment.hash ?? -1;
		const oldNode =
			hash === -1
				? undefined
				: oldNodesMap.get(renderedNodeCacheKey(comment.id, hash));
		if (oldNode) {
			nodes.push(oldNode);
			continue;
		}

		// Use mediaNode for Image/Video comment types, commentNode for Text
		const isMedia =
			comment.comment_type === ICommentType.Image ||
			comment.comment_type === ICommentType.Video;

		nodes.push({
			id: comment.id,
			type: isMedia ? "mediaNode" : "commentNode",
			position: { x: comment.coordinates[0], y: comment.coordinates[1] },
			width: comment.width ?? (isMedia ? 400 : 200),
			height: comment.height ?? (isMedia ? 300 : 80),
			zIndex: comment.z_index ?? 1,
			draggable: !(comment.is_locked ?? false),
			data: {
				label: comment.id,
				boardId: board.id,
				appId: appId,
				hash: hash,
				boardRef: boardRef,
				comment: { ...comment, is_locked: comment.is_locked ?? false },
				presignedUrl: comment.presigned_url,
				onUpsert: async (comment: IComment) => {
					const command = upsertCommentCommand({
						comment: comment,
						current_layer: currentLayer,
					});
					await executeCommand(command, false);
				},
			},
			selected: selected.has(comment.id),
		});
	}
	return { nodes, edges, cache };
}

export function handleCopy(
	nodes: any[],
	board: IBoard,
	cursorPosition?: { x: number; y: number },
	event?: ClipboardEvent,
	currentLayer?: string,
) {
	if (shouldIgnoreBoardClipboardEvent(event)) {
		return;
	}

	event?.preventDefault();
	event?.stopPropagation();

	const allLayer = Object.values(board.layers);

	const startLayer: ILayer[] = nodes
		.filter((node) => node.selected && node.type === "layerNode")
		.map((node) => node.data.layer);

	const foundLayer = new Map<string, ILayer>(
		startLayer.map((layer) => [layer.id, { ...layer, parent_id: undefined }]),
	);

	let previousSize = 0;

	while (previousSize < foundLayer.size) {
		previousSize = foundLayer.size;
		for (const layer of allLayer) {
			if (foundLayer.has(layer.id)) continue;
			if (!layer.parent_id || layer.parent_id === "") continue;
			if (foundLayer.has(layer.parent_id)) {
				foundLayer.set(layer.id, layer);
			}
		}
	}

	const selected = new Set(
		nodes.filter((node) => node.selected).map((node) => node.id),
	);
	const selectedNodes = Object.values(board.nodes)
		.filter((node) => selected.has(node.id) || foundLayer.has(node.layer ?? ""))
		.map((node) =>
			serializeNode({
				...node,
				layer:
					(node.layer ?? "") === (currentLayer ?? "") ? undefined : node.layer,
			}),
		);

	const selectedComments = Object.values(board.comments)
		.filter(
			(comment) =>
				selected.has(comment.id) || foundLayer.has(comment.layer ?? ""),
		)
		.map((comment) => ({
			...comment,
			layer:
				(comment.layer ?? "") === (currentLayer ?? "")
					? undefined
					: comment.layer,
		}));

	// Collect variables referenced by the selected nodes
	const referencedVarIds = new Set<string>();
	for (const node of selectedNodes) {
		for (const pin of Object.values(node.pins)) {
			if (pin.name === "var_ref" && pin.default_value) {
				try {
					const bytes = new Uint8Array(pin.default_value);
					const jsonStr = new TextDecoder().decode(bytes);
					const varRef = JSON.parse(jsonStr);
					if (typeof varRef === "string") {
						referencedVarIds.add(varRef);
					}
				} catch {
					// Ignore parse errors
				}
			}
		}
	}

	const selectedVariables = Object.values(board.variables).filter((v) =>
		referencedVarIds.has(v.id),
	);

	// Collect board refs used by variables and pins so schemas survive paste
	const referencedRefs: Record<string, string> = {};
	for (const v of selectedVariables) {
		if (v.schema && board.refs[v.schema]) {
			referencedRefs[v.schema] = board.refs[v.schema];
		}
	}
	for (const node of selectedNodes) {
		for (const pin of Object.values(node.pins)) {
			if (pin.schema && board.refs[pin.schema]) {
				referencedRefs[pin.schema] = board.refs[pin.schema];
			}
		}
	}

	try {
		navigator.clipboard.writeText(
			JSON.stringify(
				{
					nodes: selectedNodes,
					comments: selectedComments,
					cursorPosition,
					layers: Array.from(foundLayer.values()),
					variables: selectedVariables,
					refs: referencedRefs,
				},
				null,
				2,
			),
		);
		toastSuccess("Nodes copied to clipboard", <CopyIcon className="w-4 h-4" />);
		return;
	} catch (error) {
		toast.error("Failed to copy nodes to clipboard");
		throw error;
	}
}

export async function handlePaste(
	event: ClipboardEvent,
	cursorPosition: { x: number; y: number },
	boardId: string,
	executeCommand: (command: IGenericCommand, append?: boolean) => Promise<any>,
	currentLayer?: string,
	catalog?: INode[],
) {
	if (shouldIgnoreBoardClipboardEvent(event)) {
		return;
	}

	event.preventDefault();
	event.stopPropagation();

	// 1. Try flow-like clipboard format (copy/paste within editor)
	try {
		const clipboard = await navigator.clipboard.readText();
		const data = JSON.parse(clipboard);
		if (!data) return;
		if (!data.nodes && !data.comments) return;
		const oldPosition = data.cursorPosition;
		const rawNodes = Array.isArray(data.nodes)
			? data.nodes
			: Object.values(data.nodes ?? {});
		const nodes: any[] = rawNodes.map((node: ISerializedNode) =>
			deserializeNode(node),
		);
		const rawComments = Array.isArray(data.comments)
			? data.comments
			: Object.values(data.comments ?? {});
		const comments: any[] = rawComments;
		const rawLayers = Array.isArray(data.layers)
			? data.layers
			: Object.values(data.layers ?? {});
		const layers: ILayer[] = rawLayers;
		const rawVariables = Array.isArray(data.variables)
			? data.variables
			: Object.values(data.variables ?? {});
		const variables: IVariable[] = rawVariables;
		const refs: Record<string, string> = data.refs ?? {};

		const command = copyPasteCommand({
			original_comments: comments,
			original_nodes: nodes,
			original_layers: layers,
			original_variables: variables,
			original_refs: refs,
			new_comments: [],
			new_nodes: [],
			new_layers: [],
			current_layer: currentLayer,
			old_mouse: oldPosition ? [oldPosition.x, oldPosition.y, 0] : undefined,
			offset: [cursorPosition.x, cursorPosition.y, 0],
		});
		await executeCommand(command);
		return;
	} catch (error) {}

	// 2. Try n8n / Dify workflow paste
	try {
		const clipboard = await navigator.clipboard.readText();
		const detection = detectFormat(clipboard);
		if (detection.format !== "unknown" && detection.parsed) {
			const result =
				detection.format === "n8n"
					? translateN8n(detection.parsed as N8nWorkflow, catalog)
					: translateDify(detection.parsed as DifyWorkflow);

			const boardNodes = Object.values(result.board.nodes);
			const boardComments = Object.values(result.board.comments);
			const boardLayers = Object.values(result.board.layers);
			const boardVariables = Object.values(result.board.variables);

			if (boardNodes.length > 0) {
				const command = copyPasteCommand({
					original_nodes: boardNodes,
					original_comments: boardComments,
					original_layers: boardLayers,
					original_variables: boardVariables,
					original_refs: result.board.refs ?? {},
					new_comments: [],
					new_nodes: [],
					new_layers: [],
					current_layer: currentLayer,
					old_mouse: [0, 0, 0],
					offset: [cursorPosition.x, cursorPosition.y, 0],
				});
				await executeCommand(command);
				if (result.status === "partial") {
					toastSuccess(
						`Imported ${result.stats.totalNodes} nodes from ${detection.format} (${result.stats.todo} need manual setup)`,
						<Import className="w-4 h-4" />,
					);
				} else {
					toastSuccess(
						`Imported ${result.stats.totalNodes} nodes from ${detection.format}`,
						<Import className="w-4 h-4" />,
					);
				}
				return;
			}
		}
	} catch (error) {}

	// 3. Fallback: paste as text comment
	try {
		const clipboard = await navigator.clipboard.readText();
		const comment: IComment = {
			comment_type: ICommentType.Text,
			content: clipboard,
			coordinates: [cursorPosition.x, cursorPosition.y, 0],
			id: createId(),
			timestamp: {
				nanos_since_epoch: 0,
				secs_since_epoch: 0,
			},
		};

		const command = upsertCommentCommand({
			comment: comment,
			current_layer: currentLayer,
		});

		await executeCommand(command);
		return;
	} catch (error) {}
}
