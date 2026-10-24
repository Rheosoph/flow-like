/** Mirrors `FlowScriptChanges` (packages/core/editor/src/flow/ast/changes.rs). */
export type FlowScriptConnectionKind = "execution" | "data";

export interface IFlowScriptChangedNode {
	id: string;
	node_type: string;
	friendly_name: string;
	layer?: string;
}

/** A layer boundary pin names its layer: `node_id` is the layer id, `node_type` `layer:<type>`. */
export interface IFlowScriptEndpoint {
	node_id: string;
	node_type: string;
	pin: string;
}

export interface IFlowScriptConnection {
	kind: FlowScriptConnectionKind;
	from: IFlowScriptEndpoint;
	to: IFlowScriptEndpoint;
}

export interface IFlowScriptMovedNode {
	id: string;
	node_type: string;
	from_layer?: string;
	to_layer?: string;
}

export interface IFlowScriptValueChange {
	node_id: string;
	node_type: string;
	pin: string;
	before?: unknown;
	after?: unknown;
}

/** `key` is `""` for the node's own binding, else an output pin name. */
export interface IFlowScriptNameChange {
	node_id: string;
	key: string;
	before?: string;
	after?: string;
}

export interface IFlowScriptDescriptionChange {
	node_id: string;
	node_type: string;
	before: string;
	after: string;
}

export interface IFlowScriptChangedLayer {
	id: string;
	name: string;
	layer_type: string;
}

/** What an apply changed on the board; every list is omitted when empty. */
export interface IFlowScriptChanges {
	nodes_added?: IFlowScriptChangedNode[];
	nodes_removed?: IFlowScriptChangedNode[];
	nodes_moved?: IFlowScriptMovedNode[];
	connections_added?: IFlowScriptConnection[];
	connections_removed?: IFlowScriptConnection[];
	values_changed?: IFlowScriptValueChange[];
	names_changed?: IFlowScriptNameChange[];
	descriptions_changed?: IFlowScriptDescriptionChange[];
	variables_added?: string[];
	variables_removed?: string[];
	variables_changed?: string[];
	layers_added?: IFlowScriptChangedLayer[];
	layers_removed?: IFlowScriptChangedLayer[];
	layers_changed?: IFlowScriptChangedLayer[];
}

const endpoint = (end: IFlowScriptEndpoint) =>
	`${end.node_type}(${end.node_id}).${end.pin}`;

const connection = (edge: IFlowScriptConnection) =>
	`${edge.kind}: ${endpoint(edge.from)} -> ${endpoint(edge.to)}`;

function capped<T>(
	items: T[] | undefined,
	limit: number,
	format: (item: T) => unknown,
) {
	if (!items?.length) return undefined;
	return {
		total: items.length,
		items: items.slice(0, limit).map(format),
	};
}

/**
 * A bounded, model-facing view of an apply's changes: totals for every list plus the first
 * `limit` entries, connections spelled `kind: type(node id).pin -> type(node id).pin`.
 */
export function summarizeFlowScriptChanges(
	changes: IFlowScriptChanges | undefined,
	limit = 25,
): Record<string, unknown> | undefined {
	if (!changes) return undefined;
	const summary = Object.fromEntries(
		Object.entries({
			connections_added: capped(changes.connections_added, limit, connection),
			connections_removed: capped(
				changes.connections_removed,
				limit,
				connection,
			),
			nodes_added: capped(
				changes.nodes_added,
				limit,
				(node) => `${node.node_type}(${node.id})`,
			),
			nodes_removed: capped(
				changes.nodes_removed,
				limit,
				(node) => `${node.node_type}(${node.id})`,
			),
			nodes_moved: capped(changes.nodes_moved, limit, (node) => node),
			values_changed: capped(changes.values_changed, limit, (change) => ({
				pin: `${change.node_type}(${change.node_id}).${change.pin}`,
				before: change.before,
				after: change.after,
			})),
			names_changed: capped(changes.names_changed, limit, (change) => change),
			descriptions_changed: capped(
				changes.descriptions_changed,
				limit,
				(change) => change,
			),
			variables_added: capped(changes.variables_added, limit, (name) => name),
			variables_removed: capped(
				changes.variables_removed,
				limit,
				(name) => name,
			),
			variables_changed: capped(
				changes.variables_changed,
				limit,
				(name) => name,
			),
			layers_added: capped(changes.layers_added, limit, (layer) => layer),
			layers_removed: capped(changes.layers_removed, limit, (layer) => layer),
			layers_changed: capped(changes.layers_changed, limit, (layer) => layer),
		}).filter(([, value]) => value !== undefined),
	);
	return Object.keys(summary).length > 0 ? summary : undefined;
}
