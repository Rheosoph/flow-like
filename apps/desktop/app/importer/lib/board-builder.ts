import { createId } from "@paralleldrive/cuid2";
import type {
	IBoard,
	IComment,
	INode,
	IPin,
	IVariable,
} from "@tm9657/flow-like-ui";
import {
	ICommentType,
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
	IPinType,
	IValueType,
	IVariableType,
} from "@tm9657/flow-like-ui";
import { type ILayer, ILayerType } from "@tm9657/flow-like-ui";
import type { TranslationDiagnostic } from "./types";

const textEncoder = new TextEncoder();

export function encodeJsonDefault(value: unknown): number[] {
	return Array.from(textEncoder.encode(JSON.stringify(value)));
}

export type CatalogIndex = Map<string, INode>;

export function buildCatalogIndex(catalog: INode[]): CatalogIndex {
	const index = new Map<string, INode>();
	for (const node of catalog) {
		index.set(node.name, node);
	}
	return index;
}

export function cloneNodeFromCatalog(
	catalogIndex: CatalogIndex,
	catalogName: string,
	opts: {
		friendlyName: string;
		description?: string;
		x: number;
		y: number;
		comment?: string;
		layer?: string;
		start?: boolean;
	},
): INode | undefined {
	const template = catalogIndex.get(catalogName);
	if (!template) return undefined;

	const nodeId = createId();
	const clonedPins: Record<string, IPin> = {};
	for (const pin of Object.values(template.pins)) {
		const pinId = createId();
		clonedPins[pinId] = {
			...pin,
			id: pinId,
			connected_to: [],
			depends_on: [],
		};
	}

	return {
		...template,
		id: nodeId,
		friendly_name: opts.friendlyName,
		description: opts.description ?? template.description,
		coordinates: [opts.x, opts.y, 0],
		pins: clonedPins,
		comment: opts.comment ?? null,
		layer: opts.layer ?? null,
		start: opts.start ?? template.start ?? null,
		hash: null,
		error: null,
	};
}

export function setPinDefault(
	node: INode,
	pinName: string,
	value: unknown,
): void {
	const pin = findPinByName(node, pinName, IPinType.Input);
	if (pin) {
		pin.default_value = encodeJsonDefault(value);
	}
}

export function now(): { secs_since_epoch: number; nanos_since_epoch: number } {
	const ms = Date.now();
	return {
		secs_since_epoch: Math.floor(ms / 1000),
		nanos_since_epoch: (ms % 1000) * 1_000_000,
	};
}

export function createEmptyBoard(name: string, description: string): IBoard {
	const ts = now();
	return {
		id: createId(),
		name,
		description,
		nodes: {},
		variables: {},
		comments: {},
		layers: {},
		refs: {},
		viewport: [0, 0, 1],
		version: [0, 0, 1],
		stage: IExecutionStage.Dev,
		execution_mode: IExecutionMode.Hybrid,
		log_level: ILogLevel.Debug,
		page_ids: [],
		created_at: ts,
		updated_at: ts,
	};
}

export function createNode(opts: {
	name: string;
	friendlyName: string;
	description: string;
	category: string;
	x: number;
	y: number;
	comment?: string;
	layer?: string;
	start?: boolean;
}): INode {
	return {
		id: createId(),
		name: opts.name,
		friendly_name: opts.friendlyName,
		description: opts.description,
		category: opts.category,
		coordinates: [opts.x, opts.y, 0],
		pins: {},
		comment: opts.comment ?? null,
		layer: opts.layer ?? null,
		start: opts.start ?? null,
		icon: null,
		docs: null,
		error: null,
		event_callback: null,
		hash: null,
		long_running: null,
		scores: null,
		version: null,
		wasm: null,
	};
}

let pinIndex = 0;

export function createPin(opts: {
	name: string;
	friendlyName: string;
	description?: string;
	pinType: IPinType;
	dataType: IVariableType;
	valueType?: IValueType;
	defaultValue?: unknown;
	options?: { valid_values?: string[] };
	schema?: string;
}): IPin {
	pinIndex++;
	return {
		id: createId(),
		name: opts.name,
		friendly_name: opts.friendlyName,
		description: opts.description ?? "",
		pin_type: opts.pinType,
		data_type: opts.dataType,
		value_type: opts.valueType ?? IValueType.Normal,
		default_value:
			opts.defaultValue !== undefined
				? encodeJsonDefault(opts.defaultValue)
				: null,
		connected_to: [],
		depends_on: [],
		index: pinIndex,
		options: opts.options
			? {
					...opts.options,
					enforce_generic_value_type: null,
					enforce_schema: null,
					range: null,
					sensitive: null,
					step: null,
				}
			: null,
		schema: opts.schema ?? null,
	};
}

export function createVariable(opts: {
	name: string;
	description?: string;
	dataType: IVariableType;
	valueType?: IValueType;
	defaultValue?: unknown;
	secret?: boolean;
	exposed?: boolean;
	editable?: boolean;
}): IVariable {
	return {
		id: createId(),
		name: opts.name,
		description: opts.description ?? null,
		data_type: opts.dataType,
		value_type: opts.valueType ?? IValueType.Normal,
		default_value:
			opts.defaultValue !== undefined
				? encodeJsonDefault(opts.defaultValue)
				: null,
		secret: opts.secret ?? false,
		exposed: opts.exposed ?? false,
		editable: opts.editable ?? true,
		category: null,
		hash: null,
		schema: null,
	};
}

export function createTodoLayer(opts: {
	name: string;
	comment: string;
	x: number;
	y: number;
	parentId?: string;
}): ILayer {
	return {
		id: createId(),
		name: opts.name,
		type: ILayerType.Collapsed,
		coordinates: [opts.x, opts.y, 0],
		comment: opts.comment,
		color: "#FFA500",
		parent_id: opts.parentId ?? null,
		nodes: {},
		variables: {},
		comments: {},
		pins: {},
		error: null,
		hash: null,
		in_coordinates: [-200, 0, 0],
		out_coordinates: [200, 0, 0],
	};
}

export function createCompositionLayer(opts: {
	name: string;
	x: number;
	y: number;
}): ILayer {
	return {
		id: createId(),
		name: opts.name,
		type: ILayerType.Collapsed,
		coordinates: [opts.x, opts.y, 0],
		comment: null,
		color: "#90CAF9",
		parent_id: null,
		nodes: {},
		variables: {},
		comments: {},
		pins: {},
		error: null,
		hash: null,
		in_coordinates: [-200, 0, 0],
		out_coordinates: [200, 0, 0],
	};
}

export function createTodoComment(opts: {
	content: string;
	x: number;
	y: number;
	layer?: string;
}): IComment {
	const ts = now();
	return {
		id: createId(),
		content: opts.content,
		comment_type: ICommentType.Text,
		coordinates: [opts.x, opts.y, 0],
		timestamp: ts,
		author: "Importer",
		color: "#FFA500",
		height: null,
		width: null,
		z_index: null,
		is_locked: null,
		layer: opts.layer ?? null,
		hash: null,
	};
}

export function addExecPins(node: INode): { inPin: IPin; outPin: IPin } {
	const inPin = createPin({
		name: "exec_in",
		friendlyName: "▶",
		description: "Execution input",
		pinType: IPinType.Input,
		dataType: IVariableType.Execution,
	});
	const outPin = createPin({
		name: "exec_out",
		friendlyName: "▶",
		description: "Execution output",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[inPin.id] = inPin;
	node.pins[outPin.id] = outPin;
	return { inPin, outPin };
}

export function connectPins(
	sourceNode: INode,
	sourcePin: IPin,
	targetNode: INode,
	targetPin: IPin,
): void {
	const isExec = sourcePin.data_type === IVariableType.Execution;

	// Exec out: single connection only; Data out: multiple allowed
	if (isExec) {
		sourcePin.connected_to = [targetPin.id];
	} else {
		sourcePin.connected_to.push(targetPin.id);
	}

	// Exec in: multiple allowed; Data in: single source only
	if (isExec) {
		targetPin.depends_on.push(sourcePin.id);
	} else {
		targetPin.depends_on = [sourcePin.id];
	}
}

export function findPinByName(
	node: INode,
	name: string,
	pinType: IPinType,
): IPin | undefined {
	return Object.values(node.pins).find(
		(p) => p.name === name && p.pin_type === pinType,
	);
}

export function addNodeToBoard(board: IBoard, node: INode): void {
	board.nodes[node.id] = node;
}

export function addLayerToBoard(board: IBoard, layer: ILayer): void {
	board.layers[layer.id] = layer;
}

export function addVariableToBoard(board: IBoard, variable: IVariable): void {
	board.variables[variable.id] = variable;
}

export function addCommentToBoard(board: IBoard, comment: IComment): void {
	board.comments[comment.id] = comment;
}

/**
 * Creates bridge pins on every layer in the board for pins whose connections
 * cross the layer boundary. Must be called AFTER all connections have been
 * wired so that depends_on / connected_to are populated.
 */
export function createBridgePinsForBoard(board: IBoard): void {
	for (const layer of Object.values(board.layers)) {
		const innerNodes = Object.values(board.nodes).filter(
			(n) => n.layer === layer.id,
		);
		if (innerNodes.length === 0) continue;

		const innerPinIds = new Set<string>();
		for (const node of innerNodes) {
			for (const pinId of Object.keys(node.pins)) {
				innerPinIds.add(pinId);
			}
		}

		for (const node of innerNodes) {
			for (const pin of Object.values(node.pins)) {
				const isInput = pin.pin_type === IPinType.Input;
				const refs = isInput
					? (pin.depends_on ?? [])
					: (pin.connected_to ?? []);

				const hasExternal = refs.some((id) => !innerPinIds.has(id));

				// Only bridge: execution pins OR data pins with cross-layer connections
				const isExec = pin.data_type === IVariableType.Execution;
				if (!isExec && !hasExternal) continue;
				if (isExec && refs.length > 0 && !hasExternal) continue;

				const bridge = createPin({
					name: pin.name,
					friendlyName: pin.friendly_name ?? pin.name,
					pinType: pin.pin_type as IPinType,
					dataType: pin.data_type as IVariableType,
				});
				bridge.value_type = pin.value_type;
				if (pin.schema) bridge.schema = pin.schema;
				if (pin.options) bridge.options = pin.options;

				if (isInput) {
					bridge.connected_to = [pin.id];
					const externalDeps = refs.filter((id) => !innerPinIds.has(id));
					bridge.depends_on = externalDeps;
					pin.depends_on = [
						bridge.id,
						...refs.filter((id) => innerPinIds.has(id)),
					];
					for (const depId of externalDeps) {
						const srcPin = findPinInBoard(board, depId);
						if (srcPin) {
							srcPin.connected_to = srcPin.connected_to.map((id) =>
								id === pin.id ? bridge.id : id,
							);
						}
					}
				} else {
					bridge.depends_on = [pin.id];
					const externalTargets = refs.filter((id) => !innerPinIds.has(id));
					bridge.connected_to = externalTargets;
					pin.connected_to = [
						bridge.id,
						...refs.filter((id) => innerPinIds.has(id)),
					];
					for (const tgtId of externalTargets) {
						const tgtPin = findPinInBoard(board, tgtId);
						if (tgtPin) {
							tgtPin.depends_on = tgtPin.depends_on.map((id) =>
								id === pin.id ? bridge.id : id,
							);
						}
					}
				}

				layer.pins[bridge.id] = bridge;
			}
		}
	}
}

function findPinInBoard(board: IBoard, pinId: string): IPin | undefined {
	for (const node of Object.values(board.nodes)) {
		if (node.pins[pinId]) return node.pins[pinId];
	}
	for (const layer of Object.values(board.layers)) {
		if (layer.pins[pinId]) return layer.pins[pinId];
	}
	return undefined;
}

/**
 * Compute layer in/out coordinates based on the bounding box of inner nodes.
 * Call after all nodes have been assigned to layers.
 */
export function computeLayerCoordinates(board: IBoard): void {
	for (const layer of Object.values(board.layers)) {
		const innerNodes = Object.values(board.nodes).filter(
			(n) => n.layer === layer.id,
		);
		if (innerNodes.length === 0) continue;

		let minX = Number.POSITIVE_INFINITY;
		let maxX = Number.NEGATIVE_INFINITY;
		let minY = Number.POSITIVE_INFINITY;
		let maxY = Number.NEGATIVE_INFINITY;

		for (const node of innerNodes) {
			const [x, y] = node.coordinates ?? [0, 0];
			if (x < minX) minX = x;
			if (x > maxX) maxX = x;
			if (y < minY) minY = y;
			if (y > maxY) maxY = y;
		}

		const cx = (minX + maxX) / 2;
		const cy = (minY + maxY) / 2;
		const halfWidth = Math.max((maxX - minX) / 2 + 150, 200);

		layer.in_coordinates = [cx - halfWidth, cy, 0];
		layer.out_coordinates = [cx + halfWidth, cy, 0];
	}
}

export function warn(
	diagnostics: TranslationDiagnostic[],
	msg: string,
	nodeId?: string,
	nodeName?: string,
): void {
	diagnostics.push({ level: "warn", message: msg, nodeId, nodeName });
}

export function info(
	diagnostics: TranslationDiagnostic[],
	msg: string,
	nodeId?: string,
	nodeName?: string,
): void {
	diagnostics.push({ level: "info", message: msg, nodeId, nodeName });
}

export function diagError(
	diagnostics: TranslationDiagnostic[],
	msg: string,
	nodeId?: string,
	nodeName?: string,
): void {
	diagnostics.push({ level: "error", message: msg, nodeId, nodeName });
}
