/**
 * Turns a sync response into a full `IBoard`, starting from the board the client already holds.
 *
 * Node segments are replaced wholesale — never merged — so a node that moved between layers
 * cannot linger in the segment it left. The one exception is a *patch* (`ISyncSegment.base`),
 * which upserts/removes named nodes onto the exact segment revision the client sent; the server
 * only produces one when it could resolve that revision. Nodes of untouched segments — and, under
 * a patch, untouched nodes of the patched segment — keep their object identity, which is what lets
 * React memoisation skip them.
 */
import { boardFormatVersion } from "../board-format";
import type { IBoard } from "../schema/flow/board";
import type { INode, IPin } from "../schema/flow/node";
import { markBoardSnapshot } from "./identity";
import type {
	IBoardSyncManifest,
	IBoardSyncRequest,
	IBoardSyncResponse,
	ISyncNode,
	ISyncPin,
} from "./types";

/** Segment id of nodes without a layer. Must match `ROOT_SEGMENT` in the Rust module. */
export const ROOT_SEGMENT = "__root__";

/**
 * Ids in a sync payload are chosen by whoever wrote the board. `obj[id] = value` with an id of
 * `__proto__` would swap the map's prototype and hide the entity from every `Object.keys` walk
 * while it keeps executing, so entries are always written as own data properties.
 */
export function setEntry<T>(
	target: Record<string, T>,
	id: string,
	value: T,
): void {
	Object.defineProperty(target, id, {
		value,
		enumerable: true,
		configurable: true,
		writable: true,
	});
}

function assignEntries<T>(
	target: Record<string, T>,
	source: Record<string, T> | undefined,
): void {
	for (const [id, value] of Object.entries(source ?? {}))
		setEntry(target, id, value);
}

/** The effective layer of a node: `""` and `null`/`undefined` are the same thing to the canvas. */
export function nodeSegment(node: { layer?: string | null }): string {
	const layer = node.layer ?? "";
	return layer === "" ? ROOT_SEGMENT : layer;
}

export interface IAppliedBoardSync {
	board: IBoard;
	changes: BoardChangeSet;
	manifest: IBoardSyncManifest;
	/**
	 * Segments containing at least one node the server shipped lean but the supplied catalog
	 * could not rebuild (missing entry or different version). The caller must re-request these
	 * without hydration before handing the board out; until then those nodes carry placeholder
	 * catalog fields.
	 */
	unhydratable: Set<string>;
	/**
	 * Segments that arrived as a patch onto a revision this client does not hold (the request's
	 * token for the segment differs from the patch's `base`). Nothing was applied for them; the
	 * caller must re-request them whole. Cannot happen when the request was built from the held
	 * manifest and applied onto that same held board, but a diff must never be trusted blindly.
	 */
	unpatchable: Set<string>;
	/** `false` when no values changed and `board` is `prev` by identity. */
	changed: boolean;
}

export interface BoardChangeSet {
	meta: boolean;
	variables: boolean;
	comments: boolean;
	refs: string[];
	layers: { upserted: string[]; removed: string[] };
	nodes: { upserted: string[]; removed: string[] };
}

const segmentIndexes = new WeakMap<IBoard["nodes"], Map<string, Set<string>>>();

function segmentIndex(nodes: IBoard["nodes"]): Map<string, Set<string>> {
	let index = segmentIndexes.get(nodes);
	if (!index) {
		index = new Map();
		for (const [id, node] of Object.entries(nodes)) {
			const segment = nodeSegment(node);
			let members = index.get(segment);
			if (!members) {
				members = new Set();
				index.set(segment, members);
			}
			members.add(id);
		}
		segmentIndexes.set(nodes, index);
	}
	return index;
}

export function emptyBoardChanges(): BoardChangeSet {
	return {
		meta: false,
		variables: false,
		comments: false,
		refs: [],
		layers: { upserted: [], removed: [] },
		nodes: { upserted: [], removed: [] },
	};
}

export type CatalogByName = ReadonlyMap<string, INode>;

export function catalogByName(nodes: readonly INode[]): CatalogByName {
	const map = new Map<string, INode>();
	for (const node of nodes) map.set(node.name, node);
	return map;
}

function decodeBase64(text: string): number[] {
	const binary = atob(text);
	const bytes = new Array<number>(binary.length);
	for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
	return bytes;
}

function catalogPinsByName(node: INode | undefined): Map<string, IPin> {
	const map = new Map<string, IPin>();
	if (!node) return map;
	for (const pin of Object.values(node.pins ?? {})) map.set(pin.name, pin);
	return map;
}

function toPin(
	wire: ISyncPin,
	catalogPin: IPin | undefined,
): { pin: IPin; missing: boolean } {
	const missing = catalogPin === undefined;
	const pin: IPin = {
		id: wire.id,
		name: wire.name,
		index: wire.index,
		schema: wire.schema ?? null,
		connected_to: wire.connected_to ?? [],
		depends_on: wire.depends_on ?? [],
		default_value:
			wire.default_value == null ? null : decodeBase64(wire.default_value),
		friendly_name: wire.friendly_name ?? catalogPin?.friendly_name ?? "",
		description: wire.description ?? catalogPin?.description ?? "",
		pin_type: (wire.pin_type ?? catalogPin?.pin_type) as IPin["pin_type"],
		data_type: (wire.data_type ?? catalogPin?.data_type) as IPin["data_type"],
		value_type: (wire.value_type ??
			catalogPin?.value_type) as IPin["value_type"],
		options: wire.options ?? catalogPin?.options ?? null,
	};
	return { pin, missing };
}

/**
 * Rebuild one node. `hydratable` is `false` when the wire asked for hydration and the catalog
 * could not supply it; the returned node is then a placeholder that must be replaced.
 */
export function toNode(
	wire: ISyncNode,
	catalog: CatalogByName | undefined,
): { node: INode; hydratable: boolean } {
	const wantsHydration = wire.h === true;
	const catalogNode = wantsHydration ? catalog?.get(wire.name) : undefined;
	let hydratable =
		!wantsHydration ||
		(catalogNode !== undefined &&
			(catalogNode.version ?? null) === (wire.version ?? null));
	const catalogPins = wantsHydration
		? catalogPinsByName(catalogNode)
		: undefined;

	const pins: Record<string, IPin> = {};
	for (const [id, wirePin] of Object.entries(wire.pins ?? {})) {
		const { pin, missing } = toPin(wirePin, catalogPins?.get(wirePin.name));
		if (wantsHydration && missing) hydratable = false;
		setEntry(pins, id, pin);
	}

	const node: INode = {
		id: wire.id,
		name: wire.name,
		version: wire.version ?? null,
		coordinates: wire.coordinates ?? null,
		layer: wire.layer ?? null,
		comment: wire.comment ?? null,
		start: wire.start ?? null,
		error: wire.error ?? null,
		pins_collapsed: wire.pins_collapsed ?? null,
		auto_reroute: wire.auto_reroute ?? null,
		code_names: wire.code_names ?? null,
		hash: wire.hash ?? null,
		fn_refs: wire.fn_refs ?? null,
		wasm: wire.wasm ?? null,
		pins,
		friendly_name: wire.friendly_name,
		description: wire.description ?? catalogNode?.description ?? "",
		category: wire.category ?? catalogNode?.category ?? "",
		icon: wire.icon ?? catalogNode?.icon ?? null,
		docs: wire.docs ?? catalogNode?.docs ?? null,
		scores: wire.scores ?? catalogNode?.scores ?? null,
		long_running: wire.long_running ?? catalogNode?.long_running ?? null,
		event_callback: wire.event_callback ?? catalogNode?.event_callback ?? null,
		only_offline: wire.only_offline ?? catalogNode?.only_offline ?? false,
		oauth_providers:
			wire.oauth_providers ?? catalogNode?.oauth_providers ?? null,
		required_oauth_scopes:
			wire.required_oauth_scopes ?? catalogNode?.required_oauth_scopes ?? null,
		namespace: wire.namespace ?? catalogNode?.namespace ?? null,
		alias: wire.alias ?? catalogNode?.alias ?? null,
		receiver: wire.receiver ?? catalogNode?.receiver ?? null,
	};
	return { node, hydratable };
}

const EMPTY_BOARD_PARTS = () => ({
	nodes: {} as IBoard["nodes"],
	variables: {} as IBoard["variables"],
	comments: {} as IBoard["comments"],
	layers: {} as IBoard["layers"],
	refs: {} as IBoard["refs"],
});

/**
 * The manifest of the revision a response describes: verbatim when the server sent it whole,
 * otherwise the request's tokens with the delta and the dropped ids applied. `request` is the
 * exact request the response answers.
 */
export function resolveManifest(
	request: IBoardSyncRequest | undefined,
	response: IBoardSyncResponse,
): IBoardSyncManifest {
	if (response.manifest) return response.manifest;
	const delta = response.manifest_delta ?? {};
	const layers: Record<string, string> = { ...(request?.layers ?? {}) };
	for (const id of response.dropped_layers ?? []) delete layers[id];
	assignEntries(layers, delta.layers);
	const segments: Record<string, string> = { ...(request?.segments ?? {}) };
	for (const id of response.dropped_segments ?? []) delete segments[id];
	assignEntries(segments, delta.segments);
	return {
		meta: delta.meta ?? request?.meta ?? "",
		variables: delta.variables ?? request?.variables ?? "",
		comments: delta.comments ?? request?.comments ?? "",
		layers,
		segments,
	};
}

/**
 * @param request The request this response answers. Needed to rebuild the manifest from a delta
 * and to verify that a patch's `base` is the token the client actually sent. `undefined` (a
 * response to an empty first request) implies no held tokens.
 */
export function applyBoardSync(
	prev: IBoard | undefined,
	response: IBoardSyncResponse,
	catalog: CatalogByName | undefined,
	request?: IBoardSyncRequest,
): IAppliedBoardSync {
	const segments = response.segments ?? {};
	const dropped = response.dropped_segments ?? [];
	const changedLayers = response.layers ?? {};
	const droppedLayers = response.dropped_layers ?? [];
	const manifest = resolveManifest(request, response);
	const changes = emptyBoardChanges();
	const base = prev ?? (EMPTY_BOARD_PARTS() as unknown as IBoard);
	const unhydratable = new Set<string>();
	const unpatchable = new Set<string>();
	let nodes = base.nodes;
	const previousIndex = segmentIndex(nodes);
	let index = previousIndex;
	const touchedSegments = new Set<string>();
	const touchedNodes = new Set<string>();
	const writableMembers = (segment: string) => {
		if (index === previousIndex) index = new Map(previousIndex);
		let members = index.get(segment);
		if (!members || !touchedSegments.has(segment)) {
			members = new Set(members);
			index.set(segment, members);
			touchedSegments.add(segment);
		}
		return members;
	};
	const removeNode = (id: string) => {
		if (!Object.hasOwn(nodes, id)) return;
		const old = nodes[id];
		if (nodes === base.nodes) nodes = { ...nodes };
		delete nodes[id];
		writableMembers(nodeSegment(old)).delete(id);
		touchedNodes.add(id);
	};
	// Remove replaced segments first, so a node moving to another segment survives
	// regardless of the order in which the server serialized those segments.
	for (const segment of dropped) {
		for (const id of previousIndex.get(segment) ?? []) removeNode(id);
	}
	for (const [segmentId, segment] of Object.entries(segments)) {
		if (segment.base == null) {
			for (const id of previousIndex.get(segmentId) ?? []) removeNode(id);
		} else if (request?.segments?.[segmentId] !== segment.base) {
			unpatchable.add(segmentId);
		} else {
			for (const id of segment.removed ?? []) {
				if (Object.hasOwn(nodes, id) && nodeSegment(nodes[id]) === segmentId)
					removeNode(id);
			}
		}
	}
	for (const [segmentId, segment] of Object.entries(segments)) {
		if (unpatchable.has(segmentId)) continue;
		for (const [id, wire] of Object.entries(segment.nodes ?? {})) {
			const { node, hydratable } = toNode(wire, catalog);
			if (!hydratable) unhydratable.add(segmentId);
			if (Object.hasOwn(nodes, id))
				writableMembers(nodeSegment(nodes[id])).delete(id);
			if (nodes === base.nodes) nodes = { ...nodes };
			setEntry(nodes, id, node);
			writableMembers(nodeSegment(node)).add(id);
			touchedNodes.add(id);
		}
	}
	if (nodes !== base.nodes) segmentIndexes.set(nodes, index);
	for (const id of touchedNodes) {
		if (Object.hasOwn(nodes, id)) changes.nodes.upserted.push(id);
		else if (Object.hasOwn(base.nodes, id)) changes.nodes.removed.push(id);
	}

	let layers = base.layers;
	for (const id of droppedLayers) {
		if (!Object.hasOwn(layers, id) || Object.hasOwn(changedLayers, id))
			continue;
		if (layers === base.layers) layers = { ...layers };
		delete layers[id];
		changes.layers.removed.push(id);
	}
	for (const [id, layer] of Object.entries(changedLayers)) {
		if (Object.hasOwn(layers, id) && layers[id] === layer) continue;
		if (layers === base.layers) layers = { ...layers };
		setEntry(layers, id, layer);
		changes.layers.upserted.push(id);
	}
	let refs = base.refs ?? {};
	for (const [id, value] of Object.entries(response.refs ?? {})) {
		if (Object.hasOwn(refs, id) && refs[id] === value) continue;
		if (refs === base.refs) refs = { ...refs };
		setEntry(refs, id, value);
		changes.refs.push(id);
	}
	changes.meta = response.meta != null;
	changes.variables =
		response.variables != null && response.variables !== base.variables;
	changes.comments =
		response.comments != null && response.comments !== base.comments;
	const changed =
		!prev ||
		changes.meta ||
		changes.variables ||
		changes.comments ||
		changes.refs.length > 0 ||
		layers !== base.layers ||
		nodes !== base.nodes;
	if (!changed)
		return {
			board: base,
			manifest,
			changes,
			unhydratable,
			unpatchable,
			changed: false,
		};

	const meta = response.meta;
	const board: IBoard = {
		...base,
		format_version: boardFormatVersion(
			meta ? meta.format_version : base.format_version,
		),
		...(meta
			? {
					id: meta.id,
					name: meta.name,
					description: meta.description,
					viewport: meta.viewport,
					version: meta.version,
					stage: meta.stage,
					log_level: meta.log_level,
					execution_mode: meta.execution_mode,
					page_ids: meta.page_ids,
					hash: meta.hash ?? null,
					created_at: meta.created_at,
					updated_at: meta.updated_at,
				}
			: {}),
		nodes,
		variables: response.variables ?? base.variables ?? {},
		comments: response.comments ?? base.comments ?? {},
		layers,
		refs,
	};

	return {
		board: markBoardSnapshot(board),
		manifest,
		changes,
		unhydratable,
		unpatchable,
		changed: true,
	};
}
