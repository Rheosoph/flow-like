import {
	type IBoard,
	type IComment,
	type ILayer,
	type INode,
	type IPin,
	IPinType,
	type IVariable,
	IVariableType,
} from "../schema/flow/board";
import type {
	BoardChangeStatus,
	BoardEdgeStatus,
	IBoardChange,
	IBoardChangeDetail,
	IBoardDiff,
} from "./types";

const MOVE_EPSILON = 1;
const MASK = "••••••";

type PinOwner = { title: string; layerId?: string };
type PinRef = { pin: IPin; owner: PinOwner };

interface BoardIndex {
	board: IBoard;
	pins: Map<string, PinRef>;
	/** Input pin id → source pin ids feeding it. */
	incoming: Map<string, Set<string>>;
	edges: Set<string>;
	variables: Map<string, { variable: IVariable; layerId?: string }>;
	comments: Map<string, { comment: IComment; layerId?: string }>;
}

export const layerKey = (layerId?: string | null) =>
	layerId && layerId !== "" ? layerId : undefined;

export function nodeTitle(node: INode): string {
	return node.friendly_name?.trim() || node.name;
}

export function layerTitle(layer: ILayer): string {
	return layer.name?.trim() || layer.id;
}

function pinLabel(pin: IPin): string {
	return pin.friendly_name?.trim() || pin.name;
}

function decodeBytes(value: number[] | null | undefined): unknown {
	if (!value || value.length === 0) return undefined;
	let text: string;
	try {
		text = new TextDecoder("utf-8").decode(new Uint8Array(value));
	} catch {
		return undefined;
	}
	if (text.trim() === "") return undefined;
	try {
		return JSON.parse(text);
	} catch {
		return text;
	}
}

function stableStringify(value: unknown): string {
	if (value === undefined) return "";
	if (value === null || typeof value !== "object") return JSON.stringify(value);
	if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
	const entries = Object.entries(value as Record<string, unknown>)
		.filter(([, v]) => v !== undefined)
		.sort(([a], [b]) => a.localeCompare(b))
		.map(([k, v]) => `${JSON.stringify(k)}:${stableStringify(v)}`);
	return `{${entries.join(",")}}`;
}

export function formatBoardValue(value: unknown): string | undefined {
	if (value === undefined) return undefined;
	if (typeof value === "string") return value;
	return JSON.stringify(value);
}

function decodedValue(bytes: number[] | null | undefined) {
	const value = decodeBytes(bytes);
	return { key: stableStringify(value), display: formatBoardValue(value) };
}

function coords(value?: number[] | null): [number, number] {
	return [value?.[0] ?? 0, value?.[1] ?? 0];
}

function movedBetween(
	from?: number[] | null,
	to?: number[] | null,
): IBoardChange["move"] {
	const a = coords(from);
	const b = coords(to);
	return Math.hypot(b[0] - a[0], b[1] - a[1]) >= MOVE_EPSILON
		? { from: a, to: b }
		: undefined;
}

function indexBoard(board: IBoard): BoardIndex {
	const pins = new Map<string, PinRef>();
	const incoming = new Map<string, Set<string>>();
	const edges = new Set<string>();
	const variables = new Map<
		string,
		{ variable: IVariable; layerId?: string }
	>();
	const comments = new Map<string, { comment: IComment; layerId?: string }>();

	for (const node of Object.values(board.nodes ?? {})) {
		const owner = { title: nodeTitle(node), layerId: layerKey(node.layer) };
		for (const pin of Object.values(node.pins ?? {}))
			pins.set(pin.id, { pin, owner });
	}
	for (const layer of Object.values(board.layers ?? {})) {
		const owner = { title: layerTitle(layer), layerId: layer.id };
		for (const pin of Object.values(layer.pins ?? {}))
			pins.set(pin.id, { pin, owner });
		for (const variable of Object.values(layer.variables ?? {}))
			variables.set(variable.id, { variable, layerId: layer.id });
		for (const comment of Object.values(layer.comments ?? {}))
			comments.set(comment.id, { comment, layerId: layer.id });
	}
	for (const variable of Object.values(board.variables ?? {}))
		variables.set(variable.id, { variable });
	for (const comment of Object.values(board.comments ?? {}))
		comments.set(comment.id, { comment, layerId: layerKey(comment.layer) });

	for (const { pin } of pins.values()) {
		for (const target of pin.connected_to ?? []) {
			if (!pins.has(target)) continue;
			edges.add(`${pin.id}-${target}`);
			let sources = incoming.get(target);
			if (!sources) {
				sources = new Set();
				incoming.set(target, sources);
			}
			sources.add(pin.id);
		}
	}
	return { board, pins, incoming, edges, variables, comments };
}

function sourceLabel(index: BoardIndex, pinId: string): string {
	const ref = index.pins.get(pinId);
	if (!ref) return pinId;
	const label = pinLabel(ref.pin);
	if (ref.pin.data_type === IVariableType.Execution) {
		const generic =
			/^exec(_out)?$/i.test(ref.pin.name) ||
			/^exec(ution)?( out)?$/i.test(label);
		return generic ? ref.owner.title : `${ref.owner.title} → ${label}`;
	}
	return `${ref.owner.title} · ${label}`;
}

function sourcesLabel(
	index: BoardIndex,
	pinIds: Iterable<string>,
): string | undefined {
	const labels = [...pinIds].map((id) => sourceLabel(index, id)).sort();
	return labels.length ? labels.join(", ") : undefined;
}

function isInput(pin: IPin) {
	return pin.pin_type === IPinType.Input;
}

/** Where a node's data outputs go, for nodes a reader knows by what they feed. */
function feedsDetail(
	index: BoardIndex,
	node: INode,
	side: "before" | "after",
): IBoardChangeDetail | undefined {
	const targets: string[] = [];
	for (const pin of Object.values(node.pins ?? {})) {
		if (isInput(pin) || pin.data_type === IVariableType.Execution) continue;
		for (const target of pin.connected_to ?? []) {
			const ref = index.pins.get(target);
			if (ref) targets.push(`${ref.owner.title} · ${pinLabel(ref.pin)}`);
		}
	}
	if (!targets.length) return undefined;
	return {
		kind: "wire",
		field: "_feeds",
		label: "Feeds",
		role: "feeds",
		[side]: [...new Set(targets)].sort().join(", "),
	};
}

function inputPins(node: INode): Map<string, IPin> {
	const map = new Map<string, IPin>();
	for (const pin of Object.values(node.pins ?? {})) {
		if (isInput(pin)) map.set(pin.id, pin);
	}
	return map;
}

function variableUsers(board: IBoard): Map<string, string[]> {
	const users = new Map<string, string[]>();
	for (const node of Object.values(board.nodes ?? {})) {
		for (const pin of Object.values(node.pins ?? {})) {
			if (pin.name !== "var_ref") continue;
			const id = decodeBytes(pin.default_value);
			if (typeof id !== "string") continue;
			const list = users.get(id) ?? [];
			list.push(node.id);
			users.set(id, list);
		}
	}
	return users;
}

function pinDetails(
	base: BoardIndex,
	head: BoardIndex,
	a: INode | undefined,
	b: INode | undefined,
): IBoardChangeDetail[] {
	const details: IBoardChangeDetail[] = [];
	const aPins = a ? inputPins(a) : new Map<string, IPin>();
	const bPins = b ? inputPins(b) : new Map<string, IPin>();
	const ids = [...new Set([...aPins.keys(), ...bPins.keys()])];
	const order = (id: string) => (bPins.get(id) ?? aPins.get(id))?.index ?? 0;
	ids.sort((x, y) => order(x) - order(y));

	for (const id of ids) {
		const pa = aPins.get(id);
		const pb = bPins.get(id);
		const pin = (pb ?? pa) as IPin;
		const label = pinLabel(pin);
		const role =
			pin.data_type === IVariableType.Execution &&
			/^exec(_in)?$/i.test(pin.name)
				? "runs-after"
				: "pin";
		const inA = a
			? (base.incoming.get(id) ?? new Set<string>())
			: new Set<string>();
		const inB = b
			? (head.incoming.get(id) ?? new Set<string>())
			: new Set<string>();
		const lost = [...inA].filter((s) => !inB.has(s));
		const gained = [...inB].filter((s) => !inA.has(s));

		if (a && b) {
			if (lost.length || gained.length) {
				details.push({
					kind: "wire",
					field: pin.name,
					label,
					role,
					before: sourcesLabel(base, lost),
					after: sourcesLabel(head, gained),
				});
				continue;
			}
			if (inB.size > 0) continue;
			const va = decodedValue(pa?.default_value);
			const vb = decodedValue(pb?.default_value);
			if (va.key === vb.key) continue;
			const masked = Boolean(pin.options?.sensitive);
			details.push({
				kind: "value",
				field: pin.name,
				label,
				role,
				before: masked && va.display !== undefined ? MASK : va.display,
				after: masked && vb.display !== undefined ? MASK : vb.display,
				masked,
			});
			continue;
		}

		const side = b ? "after" : "before";
		const index = b ? head : base;
		const sources = b ? inB : inA;
		if (sources.size > 0) {
			details.push({
				kind: "wire",
				field: pin.name,
				label,
				role,
				[side]: sourcesLabel(index, sources),
			});
			continue;
		}
		const value = decodedValue(pin.default_value);
		if (value.display === undefined) continue;
		const masked = Boolean(pin.options?.sensitive);
		details.push({
			kind: "value",
			field: pin.name,
			label,
			role,
			[side]: masked ? MASK : value.display,
			masked,
		});
	}
	return details;
}

function propertyDetail(
	field: string,
	label: string,
	before: unknown,
	after: unknown,
	kind: IBoardChangeDetail["kind"] = "setting",
): IBoardChangeDetail | undefined {
	const a = stableStringify(before ?? undefined);
	const b = stableStringify(after ?? undefined);
	if (a === b) return undefined;
	return {
		kind,
		field,
		label,
		before: formatBoardValue(before ?? undefined),
		after: formatBoardValue(after ?? undefined),
	};
}

function compact<T>(items: (T | undefined)[]): T[] {
	return items.filter((item): item is T => item !== undefined);
}

function layerAncestors(board: IBoard, layerId?: string): string[] {
	const chain: string[] = [];
	const seen = new Set<string>();
	let current = layerKey(layerId);
	while (current && !seen.has(current)) {
		seen.add(current);
		chain.push(current);
		current = layerKey(board.layers?.[current]?.parent_id);
	}
	return chain;
}

function unionKeys(a?: object | null, b?: object | null): Set<string> {
	return new Set([...Object.keys(a ?? {}), ...Object.keys(b ?? {})]);
}

/** Files an edit of an item present on both sides, or nothing when it did not change. */
function recordEdit(
	ctx: DiffContext,
	change: IBoardChange,
	details: IBoardChangeDetail[],
	move: IBoardChange["move"],
) {
	if (!details.length && !move) return;
	change.details = details;
	change.move = move;
	change.status = details.length ? "changed" : "moved";
	ctx.itemStatus.set(change.id, change.status);
	ctx.push(change);
}

function nodeEditDetails(
	base: BoardIndex,
	head: BoardIndex,
	a: INode,
	b: INode,
): IBoardChangeDetail[] {
	return compact<IBoardChangeDetail>([
		propertyDetail(
			"friendly_name",
			"Name",
			a.friendly_name,
			b.friendly_name,
			"name",
		),
		propertyDetail("node_type", "Node type", a.name, b.name),
		propertyDetail("comment", "Comment", a.comment, b.comment),
		propertyDetail(
			"layer",
			"Layer",
			layerTitleOf(base.board, a.layer),
			layerTitleOf(head.board, b.layer),
		),
		...pinDetails(base, head, a, b),
	]);
}

function nodeChanges(base: BoardIndex, head: BoardIndex, ctx: DiffContext) {
	for (const id of unionKeys(base.board.nodes, head.board.nodes)) {
		const a = base.board.nodes?.[id];
		const b = head.board.nodes?.[id];
		const node = (b ?? a) as INode;
		const layerId = layerKey(node.layer);
		const change: IBoardChange = {
			key: `node:${id}`,
			kind: "node",
			id,
			status: "changed",
			title: nodeTitle(node),
			layerId,
			details: [],
			focusIds: [id],
		};

		if (!a || !b) {
			change.status = b ? "added" : "removed";
			const containerLayers = layerAncestors(
				b ? head.board : base.board,
				layerId,
			);
			const swallowedBy = containerLayers.findLast(
				(l) => ctx.layerStatus.get(l) === change.status,
			);
			ctx.itemStatus.set(id, change.status);
			if (swallowedBy) {
				if (node.name !== "reroute") ctx.bumpLayerNodes(swallowedBy);
				continue;
			}
			change.details = pinDetails(base, head, a, b);
			if (!change.details.some((d) => d.role === "runs-after")) {
				const feeds = b
					? feedsDetail(head, b, "after")
					: feedsDetail(base, node, "before");
				if (feeds) change.details.push(feeds);
			}
			ctx.push(change);
			continue;
		}

		const move =
			layerKey(a.layer) === layerKey(b.layer)
				? movedBetween(a.coordinates, b.coordinates)
				: undefined;
		recordEdit(ctx, change, nodeEditDetails(base, head, a, b), move);
	}
}

function layerTitleOf(
	board: IBoard,
	layerId?: string | null,
): string | undefined {
	const id = layerKey(layerId);
	if (!id) return undefined;
	const layer = board.layers?.[id];
	return layer ? layerTitle(layer) : id;
}

function layerPinSignature(layer: ILayer): Map<string, string> {
	const map = new Map<string, string>();
	for (const pin of Object.values(layer.pins ?? {})) {
		map.set(
			pin.id,
			`${pin.pin_type}:${pinLabel(pin)}:${pin.data_type}:${pin.value_type}`,
		);
	}
	return map;
}

function layerInterfaceDetails(a: ILayer, b: ILayer): IBoardChangeDetail[] {
	const aPins = layerPinSignature(a);
	const bPins = layerPinSignature(b);
	const details: IBoardChangeDetail[] = [];
	const describe = (layer: ILayer, pinId: string) =>
		`${pinLabel(layer.pins[pinId])} (${layer.pins[pinId].data_type})`;
	for (const pinId of new Set([...aPins.keys(), ...bPins.keys()])) {
		if (aPins.get(pinId) === bPins.get(pinId)) continue;
		const pin = b.pins?.[pinId] ?? a.pins?.[pinId];
		if (!pin) continue;
		details.push({
			kind: "setting",
			field: `${isInput(pin) ? "input" : "output"}:${pinId}`,
			label: isInput(pin) ? "Input" : "Output",
			before: aPins.has(pinId) ? describe(a, pinId) : undefined,
			after: bPins.has(pinId) ? describe(b, pinId) : undefined,
		});
	}
	return details;
}

function layerEditDetails(
	base: BoardIndex,
	head: BoardIndex,
	a: ILayer,
	b: ILayer,
): IBoardChangeDetail[] {
	return compact<IBoardChangeDetail>([
		propertyDetail("name", "Name", a.name, b.name, "name"),
		propertyDetail("type", "Type", a.type, b.type),
		propertyDetail("comment", "Comment", a.comment, b.comment),
		propertyDetail("category", "Folder", a.category, b.category),
		propertyDetail(
			"parent_id",
			"Parent",
			layerTitleOf(base.board, a.parent_id),
			layerTitleOf(head.board, b.parent_id),
		),
		propertyDetail(
			"cache",
			"Result caching",
			a.cache?.enabled ?? false,
			b.cache?.enabled ?? false,
		),
		...layerInterfaceDetails(a, b),
	]);
}

function layerChanges(base: BoardIndex, head: BoardIndex, ctx: DiffContext) {
	const ids = unionKeys(base.board.layers, head.board.layers);
	for (const id of ids) {
		const a = base.board.layers?.[id];
		const b = head.board.layers?.[id];
		if (a && b) continue;
		ctx.layerStatus.set(id, b ? "added" : "removed");
	}

	for (const id of ids) {
		const a = base.board.layers?.[id];
		const b = head.board.layers?.[id];
		const layer = (b ?? a) as ILayer;
		const parentId = layerKey(layer.parent_id);
		const change: IBoardChange = {
			key: `layer:${id}`,
			kind: "layer",
			id,
			status: "changed",
			title: layerTitle(layer),
			layerId: parentId,
			details: [],
			focusIds: [id],
		};
		if (!a || !b) {
			change.status = b ? "added" : "removed";
			ctx.itemStatus.set(id, change.status);
			const parents = layerAncestors(b ? head.board : base.board, parentId);
			if (parents.some((p) => ctx.layerStatus.get(p) === change.status))
				continue;
			change.details = [
				{
					kind: "setting",
					field: "type",
					label: "Type",
					[b ? "after" : "before"]: layer.type,
				},
			];
			ctx.layerChanges.set(id, change);
			ctx.push(change);
			continue;
		}

		const move =
			layerKey(a.parent_id) === layerKey(b.parent_id)
				? movedBetween(a.coordinates, b.coordinates)
				: undefined;
		recordEdit(ctx, change, layerEditDetails(base, head, a, b), move);
	}
}

function variableDisplay(variable: IVariable, secret: boolean) {
	const decoded = decodedValue(variable.default_value);
	return secret && decoded.display !== undefined
		? { ...decoded, display: MASK }
		: decoded;
}

function variableEditDetails(
	a: IVariable,
	b: IVariable,
	secret: boolean,
): IBoardChangeDetail[] {
	const va = variableDisplay(a, secret);
	const vb = variableDisplay(b, secret);
	return compact<IBoardChangeDetail>([
		propertyDetail("name", "Name", a.name, b.name, "name"),
		propertyDetail("data_type", "Type", typeLabel(a), typeLabel(b)),
		va.key !== vb.key
			? {
					kind: "value",
					field: "default_value",
					label: "Default value",
					before: va.display,
					after: vb.display,
					masked: secret,
				}
			: undefined,
		propertyDetail("exposed", "Exposed", a.exposed, b.exposed),
		propertyDetail("editable", "Editable", a.editable, b.editable),
		propertyDetail("secret", "Secret", a.secret, b.secret),
		propertyDetail(
			"runtime_configured",
			"Set per user",
			a.runtime_configured ?? false,
			b.runtime_configured ?? false,
		),
		propertyDetail("description", "Description", a.description, b.description),
	]);
}

function variableChanges(base: BoardIndex, head: BoardIndex, ctx: DiffContext) {
	const ids = new Set([...base.variables.keys(), ...head.variables.keys()]);
	const headUsers = variableUsers(head.board);
	const baseUsers = variableUsers(base.board);
	for (const id of ids) {
		const a = base.variables.get(id);
		const b = head.variables.get(id);
		const entry = (b ?? a) as { variable: IVariable; layerId?: string };
		const variable = entry.variable;
		const change: IBoardChange = {
			key: `variable:${id}`,
			kind: "variable",
			id,
			status: "changed",
			title: variable.name,
			layerId: entry.layerId,
			details: [],
			focusIds: (b ? headUsers : baseUsers).get(id) ?? [],
		};
		const secret = Boolean(a?.variable.secret || b?.variable.secret);

		if (!a || !b) {
			change.status = b ? "added" : "removed";
			const side = b ? "after" : "before";
			const value = variableDisplay(variable, secret);
			change.details = compact<IBoardChangeDetail>([
				{
					kind: "setting",
					field: "data_type",
					label: "Type",
					[side]: typeLabel(variable),
				},
				value.display !== undefined
					? {
							kind: "value",
							field: "default_value",
							label: "Default value",
							[side]: value.display,
							masked: secret,
						}
					: undefined,
			]);
			ctx.push(change);
			continue;
		}

		const details = variableEditDetails(a.variable, b.variable, secret);
		if (!details.length) continue;
		change.details = details;
		ctx.push(change);
	}
}

function typeLabel(variable: IVariable): string {
	return variable.value_type && variable.value_type !== "Normal"
		? `${variable.data_type} ${variable.value_type}`
		: variable.data_type;
}

function commentExcerpt(comment: IComment): string {
	const text = (comment.content ?? "").replace(/\s+/g, " ").trim();
	if (!text) return comment.comment_type;
	return text.length > 60 ? `${text.slice(0, 57)}…` : text;
}

function commentChanges(base: BoardIndex, head: BoardIndex, ctx: DiffContext) {
	const ids = new Set([...base.comments.keys(), ...head.comments.keys()]);
	for (const id of ids) {
		const a = base.comments.get(id)?.comment;
		const b = head.comments.get(id)?.comment;
		const entry = (head.comments.get(id) ?? base.comments.get(id)) as {
			comment: IComment;
			layerId?: string;
		};
		const change: IBoardChange = {
			key: `comment:${id}`,
			kind: "comment",
			id,
			status: "changed",
			title: commentExcerpt(entry.comment),
			layerId: entry.layerId,
			details: [],
			focusIds: [id],
		};
		if (!a || !b) {
			change.status = b ? "added" : "removed";
			if (
				layerAncestors(b ? head.board : base.board, entry.layerId).some(
					(l) => ctx.layerStatus.get(l) === change.status,
				)
			)
				continue;
			const side = b ? "after" : "before";
			change.details = [
				{
					kind: "value",
					field: "content",
					label: "Text",
					[side]: entry.comment.content,
				},
			];
			ctx.itemStatus.set(id, change.status);
			ctx.push(change);
			continue;
		}
		const details = compact<IBoardChangeDetail>([
			propertyDetail("content", "Text", a.content, b.content, "value"),
			propertyDetail("color", "Color", a.color, b.color),
		]);
		recordEdit(
			ctx,
			change,
			details,
			movedBetween(a.coordinates, b.coordinates),
		);
	}
}

function boardChanges(base: IBoard, head: IBoard, ctx: DiffContext) {
	const details = compact<IBoardChangeDetail>([
		propertyDetail("name", "Name", base.name, head.name, "name"),
		propertyDetail(
			"description",
			"Description",
			base.description,
			head.description,
		),
		propertyDetail(
			"execution_mode",
			"Execution mode",
			base.execution_mode,
			head.execution_mode,
		),
		propertyDetail("log_level", "Log level", base.log_level, head.log_level),
	]);
	if (!details.length) return;
	ctx.push({
		key: "board:settings",
		kind: "board",
		id: "settings",
		status: "changed",
		title: head.name || base.name,
		details,
		focusIds: [],
	});
}

interface DiffContext {
	changes: IBoardChange[];
	itemStatus: Map<string, BoardChangeStatus>;
	layerStatus: Map<string, "added" | "removed">;
	layerChanges: Map<string, IBoardChange>;
	layerNodeCounts: Map<string, number>;
	push(change: IBoardChange): void;
	bumpLayerNodes(layerId: string): void;
}

const KIND_ORDER: Record<IBoardChange["kind"], number> = {
	node: 0,
	layer: 1,
	comment: 2,
	variable: 3,
	board: 4,
};

function sortChanges(changes: IBoardChange[], head: IBoard, base: IBoard) {
	const depth = (layerId?: string) =>
		layerAncestors(head, layerId).length ||
		layerAncestors(base, layerId).length;
	const position = (c: IBoardChange): [number, number] => {
		if (c.move) return c.move.to;
		if (c.kind === "node")
			return coords((head.nodes?.[c.id] ?? base.nodes?.[c.id])?.coordinates);
		if (c.kind === "layer")
			return coords((head.layers?.[c.id] ?? base.layers?.[c.id])?.coordinates);
		return [0, 0];
	};
	return changes.sort((x, y) => {
		const kx = x.kind === "board" || x.kind === "variable" ? 1 : 0;
		const ky = y.kind === "board" || y.kind === "variable" ? 1 : 0;
		if (kx !== ky) return kx - ky;
		const dx = depth(x.layerId) - depth(y.layerId);
		if (dx) return dx;
		const lx = x.layerId ?? "";
		const ly = y.layerId ?? "";
		if (lx !== ly) return lx.localeCompare(ly);
		if (KIND_ORDER[x.kind] !== KIND_ORDER[y.kind])
			return KIND_ORDER[x.kind] - KIND_ORDER[y.kind];
		const [px, py] = position(x);
		const [qx, qy] = position(y);
		return px - qx || py - qy || x.title.localeCompare(y.title);
	});
}

/**
 * Structural difference between two states of the same board: what someone reading the
 * canvas would call a change. Pins that only appear through catalog refreshes (no value,
 * no wire) and internal fields (hashes, scores, timestamps) are ignored on purpose.
 */
export function diffBoards(base: IBoard, head: IBoard): IBoardDiff {
	const baseIndex = indexBoard(base);
	const headIndex = indexBoard(head);
	const ctx: DiffContext = {
		changes: [],
		itemStatus: new Map(),
		layerStatus: new Map(),
		layerChanges: new Map(),
		layerNodeCounts: new Map(),
		push(change) {
			this.changes.push(change);
		},
		bumpLayerNodes(layerId) {
			this.layerNodeCounts.set(
				layerId,
				(this.layerNodeCounts.get(layerId) ?? 0) + 1,
			);
		},
	};

	layerChanges(baseIndex, headIndex, ctx);
	nodeChanges(baseIndex, headIndex, ctx);
	commentChanges(baseIndex, headIndex, ctx);
	variableChanges(baseIndex, headIndex, ctx);
	boardChanges(base, head, ctx);

	for (const [layerId, count] of ctx.layerNodeCounts) {
		const change = ctx.layerChanges.get(layerId);
		if (!change) continue;
		const side = change.status === "added" ? "after" : "before";
		change.details.push({
			kind: "setting",
			field: "nodes",
			label: "Nodes",
			[side]: String(count),
		});
	}

	const edgeStatus = new Map<string, BoardEdgeStatus>();
	for (const edge of headIndex.edges)
		if (!baseIndex.edges.has(edge)) edgeStatus.set(edge, "added");
	for (const edge of baseIndex.edges)
		if (!headIndex.edges.has(edge)) edgeStatus.set(edge, "removed");

	const changes = sortChanges(ctx.changes, head, base);
	const counts: Record<BoardChangeStatus, number> = {
		added: 0,
		removed: 0,
		changed: 0,
		moved: 0,
	};
	for (const change of changes) counts[change.status]++;

	const layersWithChanges = new Set<string>();
	for (const change of changes) {
		if (change.status === "moved") continue;
		const board = change.status === "removed" ? base : head;
		for (const layerId of layerAncestors(board, change.layerId))
			layersWithChanges.add(layerId);
	}

	return {
		changes,
		counts,
		logicCount: changes.length - counts.moved,
		byKey: new Map(changes.map((change) => [change.key, change])),
		itemStatus: ctx.itemStatus,
		edgeStatus,
		layersWithChanges,
	};
}
