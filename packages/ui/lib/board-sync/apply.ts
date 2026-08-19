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
import type { IBoard } from "../schema/flow/board";
import type { INode, IPin } from "../schema/flow/node";
import type {
	IBoardSyncManifest,
	IBoardSyncRequest,
	IBoardSyncResponse,
	ISyncNode,
	ISyncPin,
} from "./types";

/** Segment id of nodes without a layer. Must match `ROOT_SEGMENT` in the Rust module. */
export const ROOT_SEGMENT = "__root__";

/** The effective layer of a node: `""` and `null`/`undefined` are the same thing to the canvas. */
export function nodeSegment(node: { layer?: string | null }): string {
	const layer = node.layer ?? "";
	return layer === "" ? ROOT_SEGMENT : layer;
}

export interface IAppliedBoardSync {
	board: IBoard;
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
	/** `false` when the response carried no part at all and `board` is `prev` by identity. */
	changed: boolean;
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
		pins[id] = pin;
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
	Object.assign(layers, delta.layers ?? {});
	const segments: Record<string, string> = { ...(request?.segments ?? {}) };
	for (const id of response.dropped_segments ?? []) delete segments[id];
	Object.assign(segments, delta.segments ?? {});
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
	const changed =
		response.meta != null ||
		response.variables != null ||
		response.comments != null ||
		Object.keys(changedLayers).length > 0 ||
		droppedLayers.length > 0 ||
		Object.keys(segments).length > 0 ||
		dropped.length > 0;

	if (prev && !changed) {
		return {
			board: prev,
			manifest,
			unhydratable: new Set(),
			unpatchable: new Set(),
			changed: false,
		};
	}

	const base = prev ?? (EMPTY_BOARD_PARTS() as unknown as IBoard);
	const unhydratable = new Set<string>();
	const unpatchable = new Set<string>();

	// Segments split three ways: replaced wholesale (or dropped), patched onto the held revision,
	// and untouched. A patch whose base is not the token this client sent is refused whole — it
	// would be applied onto nodes it was not computed against.
	const replaced = new Set<string>(dropped);
	const patched = new Map<string, Set<string>>();
	for (const [segmentId, segment] of Object.entries(segments)) {
		if (segment.base == null) {
			replaced.add(segmentId);
			continue;
		}
		if (request?.segments?.[segmentId] !== segment.base) {
			unpatchable.add(segmentId);
			continue;
		}
		patched.set(segmentId, new Set(segment.removed ?? []));
	}

	// Nodes: keep every node whose segment the server did not touch, keep patched segments'
	// nodes unless removed or upserted, then take replaced segments verbatim. Segment membership
	// of a *previous* node is judged by its own layer, so a node that left a segment disappears
	// with that segment's replacement (or its patch's `removed` list).
	const nodes: IBoard["nodes"] = {};
	for (const [id, node] of Object.entries(prev?.nodes ?? {})) {
		const segmentId = nodeSegment(node);
		if (replaced.has(segmentId)) continue;
		const removed = patched.get(segmentId);
		if (removed?.has(id)) continue;
		nodes[id] = node;
	}
	for (const [segmentId, segment] of Object.entries(segments)) {
		if (unpatchable.has(segmentId)) continue;
		for (const [id, wire] of Object.entries(segment.nodes ?? {})) {
			const { node, hydratable } = toNode(wire, catalog);
			if (!hydratable) unhydratable.add(segmentId);
			nodes[id] = node;
		}
	}

	// Layer definitions are independent records keyed by id, so unlike node segments they merge:
	// take the changed ones, drop the removed ones, keep the rest by identity.
	const layers: IBoard["layers"] = {};
	for (const [id, layer] of Object.entries(prev?.layers ?? {})) {
		if (!droppedLayers.includes(id) && !(id in changedLayers))
			layers[id] = layer;
	}
	Object.assign(layers, changedLayers);

	const meta = response.meta;
	const board: IBoard = {
		...base,
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
		// Refs are content-addressed: a key never changes meaning, so upserting is the merge.
		refs: response.refs
			? { ...(base.refs ?? {}), ...response.refs }
			: (base.refs ?? {}),
	};

	return { board, manifest, unhydratable, unpatchable, changed: true };
}
